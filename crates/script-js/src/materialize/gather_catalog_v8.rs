//! The gather catalog the rs2b0t script API publishes: the woodcutting, mining and fishing camp tables, the
//! fishing methods and gear vendors, the fish-camp cook surfaces, and the fishing gear calculations.
//!
//! Tables are immutable Rust statics (`api::gather_camps`, `api::fishing_gear`, `api::cook_locations`). An
//! isolate materializes each exported table once, from a cached object shape (an `ObjectTemplate` per own-key
//! list, so every object has fast properties without a JavaScript path warming it), and every later read returns
//! that one object by identity. The calculations are `script::fishing_gear` over caller-owned pieces: the shim
//! makes one native call per member, and this module calls the script's own count functions where the frozen
//! loops would reach them.

use super::callback_v8::{self as cb, JsResult};
use crate::compat::cook_locations as cook_pairing;
use api::cook_locations::{self as cook, CookRole, FishCookSurface};
use api::fishing_gear::{self, FishingShop, FishingVendor};
use api::gather_camps::{self, CampKey, GatherCamp};
use api::selected::FactError;
use api::snapshot::WorldTile;
use rustyscript::Runtime;
use script::fishing_gear::{self as gear, BuyPlan, Count, GearHost, Item};
use script::gather_locations::js_max;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

// Constant tables, evaluated once per module (the arguments after the op are the `Tile` class).
const WOODCUTTING_TABLE: i32 = 0;
const MINING_TABLE: i32 = 1;
const FISHING_TABLE: i32 = 2;
const SHILO_VENDOR: i32 = 3;
const METHODS_TABLE: i32 = 4;
const GERRANT_VENDOR: i32 = 5;
const HARRY_VENDOR: i32 = 6;
const MINING_LABELS: i32 = 7;
const METHOD_OPTIONS: i32 = 8;
const GEAR_NAMES: i32 = 9;
const WHIRLPOOL_IDS: i32 = 10;
const SHOP_COSTS: i32 = 11;
const GERRANT_ONLY: i32 = 12;
const COOK_SURFACES: i32 = 13;
// Members.
const MINING_LABEL: i32 = 20;
const RESOLVE_METHOD: i32 = 21;
const KEEP_NAMES: i32 = 22;
const GEAR_LABEL: i32 = 23;
const HAS_GEAR: i32 = 24;
const MISSING_GEAR: i32 = 25;
const RESTOCK_PLAN: i32 = 26;
const SPOT_MATCHES: i32 = 27;
const SHOP_COST: i32 = 28;
const VENDOR_FOR: i32 = 29;
const IS_BAIT: i32 = 30;
const WITH_BAIT: i32 = 31;
const PLAN_BUYS: i32 = 32;
const PLAN_ACQUIRE: i32 = 33;
const PLANS_COST: i32 = 34;
const SHOP_CART: i32 = 35;
const SHOPABLE_MISSING: i32 = 36;
const COOK_SURFACE_FOR: i32 = 40;
const RESOLVE_COOK: i32 = 41;

const SLOT_WOODCUTTING: usize = 0;
const SLOT_MINING: usize = 1;
const SLOT_FISHING: usize = 2;
const SLOT_COOK_SURFACES: usize = 3;
const SLOT_METHODS: usize = 4;
const SLOT_GERRANT: usize = 5;
const SLOT_HARRY: usize = 6;
const SLOT_SHILO: usize = 7;
const SLOTS: usize = 8;

/// Per-isolate state: one object template per own-key list, the `Tile` class, and the materialized tables.
/// It holds no per-bot rows beyond the tables themselves; stopping the isolate drops every `Global` on its owning
/// thread.
#[derive(Default)]
struct Catalog {
    shapes: HashMap<Box<[&'static str]>, v8::Global<v8::ObjectTemplate>>,
    tile: Option<v8::Global<v8::Value>>,
    values: [Option<v8::Global<v8::Value>>; SLOTS],
}

type Shared = Rc<RefCell<Catalog>>;

pub(crate) fn install(runtime: &mut Runtime) -> Result<(), String> {
    if !runtime
        .deno_runtime()
        .v8_isolate()
        .set_slot(Shared::default())
    {
        return Err("gather catalog cache already installed".into());
    }
    cb::install(runtime, "__rs2b0t_gather_catalog", catalog_callback)
}

fn catalog_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = dispatch(scope, &args);
    cb::finish(scope, rv, result);
}

fn shared<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, Shared> {
    match scope.get_slot::<Shared>().cloned() {
        Some(shared) => Ok(shared),
        None => Err(cb::not_impl(scope, "gather catalog")),
    }
}

type Value<'s> = v8::Local<'s, v8::Value>;

fn undefined<'s>(scope: &mut v8::HandleScope<'s>) -> Value<'s> {
    v8::undefined(scope).into()
}

fn int<'s>(scope: &mut v8::HandleScope<'s>, value: i32) -> Value<'s> {
    v8::Integer::new(scope, value).into()
}

fn boolean<'s>(scope: &mut v8::HandleScope<'s>, value: bool) -> Value<'s> {
    v8::Boolean::new(scope, value).into()
}

fn array<'s>(scope: &mut v8::HandleScope<'s>, items: &[Value<'s>]) -> Value<'s> {
    v8::Array::new_with_elements(scope, items).into()
}

fn strings<'s, 'a>(
    scope: &mut v8::HandleScope<'s>,
    items: impl IntoIterator<Item = &'a str>,
) -> Value<'s> {
    let items: Vec<Value<'s>> = items
        .into_iter()
        .map(|item| cb::string(scope, item))
        .collect();
    array(scope, &items)
}

fn optional<'s, T>(
    scope: &mut v8::HandleScope<'s>,
    value: Option<T>,
    build: impl FnOnce(&mut v8::HandleScope<'s>, T) -> Value<'s>,
) -> Value<'s> {
    match value {
        Some(value) => build(scope, value),
        None => undefined(scope),
    }
}

fn key<'s>(scope: &mut v8::HandleScope<'s>, name: &str) -> v8::Local<'s, v8::String> {
    v8::String::new_from_utf8(scope, name.as_bytes(), v8::NewStringType::Internalized)
        .unwrap_or_else(|| v8::String::empty(scope))
}

/// A plain object with these own keys in this order, built from the cached template for the key list.
fn object<'s>(
    scope: &mut v8::HandleScope<'s>,
    keys: &[&'static str],
    values: &[Value<'s>],
) -> JsResult<'s, Value<'s>> {
    debug_assert_eq!(keys.len(), values.len());
    let shared = shared(scope)?;
    let cached = shared
        .borrow()
        .shapes
        .get(keys)
        .map(|template| v8::Local::new(scope, template));
    let template = match cached {
        Some(template) => template,
        None => {
            let template = shape(scope, keys);
            let global = v8::Global::new(scope, template);
            shared.borrow_mut().shapes.insert(keys.into(), global);
            template
        }
    };
    populate(scope, template, keys, values)
}

/// An object template whose own keys are these names, in this order.
fn shape<'s>(scope: &mut v8::HandleScope<'s>, keys: &[&str]) -> v8::Local<'s, v8::ObjectTemplate> {
    let template = v8::ObjectTemplate::new(scope);
    let placeholder: v8::Local<v8::Data> = v8::undefined(scope).into();
    for name in keys {
        let name = key(scope, name);
        template.set(name.into(), placeholder);
    }
    template
}

/// One instance of `template`, each own key defined as a data property (the way a spread defines them).
fn populate<'s>(
    scope: &mut v8::HandleScope<'s>,
    template: v8::Local<'s, v8::ObjectTemplate>,
    keys: &[&str],
    values: &[Value<'s>],
) -> JsResult<'s, Value<'s>> {
    let Some(row) = template.new_instance(scope) else {
        return Err(cb::type_error(
            scope,
            "gather catalog: object allocation failed",
        ));
    };
    for (name, value) in keys.iter().zip(values) {
        let name = key(scope, name);
        if !cb::catching(scope, |s| row.create_data_property(s, name.into(), *value))? {
            return Err(cb::type_error(
                scope,
                "gather catalog: could not set a property",
            ));
        }
    }
    Ok(row.into())
}

fn remember_tile<'s>(scope: &mut v8::HandleScope<'s>, ctor: Value<'s>) -> JsResult<'s, ()> {
    if v8::Local::<v8::Function>::try_from(ctor).is_err() {
        return Ok(());
    }
    let global = v8::Global::new(scope, ctor);
    shared(scope)?.borrow_mut().tile = Some(global);
    Ok(())
}

fn tile<'s>(scope: &mut v8::HandleScope<'s>, at: WorldTile) -> JsResult<'s, Value<'s>> {
    let ctor = {
        let shared = shared(scope)?;
        let held = shared
            .borrow()
            .tile
            .as_ref()
            .map(|ctor| v8::Local::new(scope, ctor));
        held
    };
    let Some(ctor) = ctor else {
        return Err(cb::type_error(
            scope,
            "gather catalog: Tile class not registered",
        ));
    };
    let args = [int(scope, at.x), int(scope, at.z), int(scope, at.level)];
    cb::construct(scope, ctor, &args, "Tile")
}

fn tiles<'s>(scope: &mut v8::HandleScope<'s>, rows: &[WorldTile]) -> JsResult<'s, Value<'s>> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(tile(scope, *row)?);
    }
    Ok(array(scope, &out))
}

/// One materialized value per slot and isolate, returned by identity afterwards.
fn cached<'s>(
    scope: &mut v8::HandleScope<'s>,
    slot: usize,
    build: impl FnOnce(&mut v8::HandleScope<'s>) -> JsResult<'s, Value<'s>>,
) -> JsResult<'s, Value<'s>> {
    let shared = shared(scope)?;
    let held = shared.borrow().values[slot]
        .as_ref()
        .map(|global| v8::Local::new(scope, global));
    if let Some(value) = held {
        return Ok(value);
    }
    let value = build(scope)?;
    let global = v8::Global::new(scope, value);
    shared.borrow_mut().values[slot] = Some(global);
    Ok(value)
}

/// What one camp row is built from: the generated literal plus what `withCampCook` derived for fishing.
struct RowView<'a> {
    camp: &'a GatherCamp,
    keys: &'a [CampKey],
    range: Option<usize>,
    obstacles: Option<&'a [&'static str]>,
    notes: Option<&'a str>,
}

fn bait_vendor<'s>(
    scope: &mut v8::HandleScope<'s>,
    vendor: &gather_camps::BaitVendor,
) -> JsResult<'s, Value<'s>> {
    let values = [
        cb::string(scope, vendor.keeper),
        tile(scope, vendor.stand)?,
        int(scope, vendor.price),
        cb::string(scope, vendor.item),
    ];
    object(scope, &["keeper", "stand", "price", "item"], &values)
}

/// A table the build lacks (no local game data) refuses as a `TypeError` naming the table.
fn facts<'s, T>(scope: &mut v8::HandleScope<'s>, result: Result<T, FactError>) -> JsResult<'s, T> {
    result.map_err(|error| cb::type_error(scope, &format!("gather catalog: {error:?}")))
}

/// One camp object. `banks` shares one `Tile` per distinct bank stand within a table, as the upstream `BANK`
/// constants do.
fn camp_row<'s>(
    scope: &mut v8::HandleScope<'s>,
    banks: &mut Vec<(WorldTile, Value<'s>)>,
    row: &RowView<'_>,
) -> JsResult<'s, Value<'s>> {
    let camp = row.camp;
    let mut names = Vec::with_capacity(row.keys.len());
    let mut values = Vec::with_capacity(row.keys.len());
    for key in row.keys {
        names.push(key.as_str());
        let value = match key {
            CampKey::Name => cb::string(scope, camp.name),
            CampKey::Spot => tile(scope, camp.spot)?,
            CampKey::BankStand => match banks.iter().find(|(at, _)| *at == camp.bank_stand) {
                Some((_, shared)) => *shared,
                None => {
                    let created = tile(scope, camp.bank_stand)?;
                    banks.push((camp.bank_stand, created));
                    created
                }
            },
            CampKey::BoothName => optional(scope, camp.booth_name, |s, v| cb::string(s, v)),
            CampKey::BoothOp => optional(scope, camp.booth_op, |s, v| cb::string(s, v)),
            CampKey::Verified => boolean(scope, camp.verified),
            CampKey::Obstacles => match row.obstacles {
                Some(obstacles) => strings(scope, obstacles.iter().copied()),
                None => undefined(scope),
            },
            CampKey::CampRadius => optional(scope, camp.camp_radius, |s, v| int(s, v)),
            CampKey::ChaseRadius => optional(scope, camp.chase_radius, |s, v| int(s, v)),
            CampKey::Resources => match camp.resources {
                Some(resources) => strings(scope, resources.iter().copied()),
                None => undefined(scope),
            },
            CampKey::AvoidSpots => match camp.avoid_spots {
                Some(spots) => tiles(scope, spots)?,
                None => undefined(scope),
            },
            CampKey::Sweep => match camp.sweep {
                Some(spots) => tiles(scope, spots)?,
                None => undefined(scope),
            },
            CampKey::BaitVendor => match &camp.bait_vendor {
                Some(vendor) => bait_vendor(scope, vendor)?,
                None => undefined(scope),
            },
            CampKey::Notes => optional(scope, row.notes, |s, v| cb::string(s, v)),
            CampKey::RecommendedCombat => {
                optional(scope, camp.recommended_combat, |s, v| int(s, v))
            }
            CampKey::RangeStand => {
                let Some(index) = row.range else {
                    return Err(cb::type_error(
                        scope,
                        "gather catalog: rangeStand without a surface",
                    ));
                };
                let object = surface_by_index(scope, index)?;
                cb::get(scope, object, "stand")?
            }
            CampKey::RangeName => match row.range {
                Some(index) => {
                    let surfaces = facts(scope, cook::fish_camp_cook_surfaces())?;
                    let Some(surface) = surfaces.get(index) else {
                        return Err(cb::type_error(
                            scope,
                            "gather catalog: rangeName without a surface",
                        ));
                    };
                    cb::string(scope, surface.loc_name)
                }
                None => undefined(scope),
            },
        };
        values.push(value);
    }
    object(scope, &names, &values)
}

fn camp_table<'s>(
    scope: &mut v8::HandleScope<'s>,
    rows: &[RowView<'_>],
) -> JsResult<'s, Value<'s>> {
    let mut banks = Vec::new();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(camp_row(scope, &mut banks, row)?);
    }
    Ok(array(scope, &out))
}

fn plain_rows(camps: &'static [GatherCamp]) -> Vec<RowView<'static>> {
    camps
        .iter()
        .map(|camp| RowView {
            camp,
            keys: camp.keys,
            range: None,
            obstacles: camp.obstacles,
            notes: camp.notes,
        })
        .collect()
}

fn woodcutting_table<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, Value<'s>> {
    cached(scope, SLOT_WOODCUTTING, |scope| {
        let camps = facts(scope, gather_camps::woodcutting_camps())?;
        camp_table(scope, &plain_rows(camps))
    })
}

fn mining_table<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, Value<'s>> {
    cached(scope, SLOT_MINING, |scope| {
        let camps = facts(scope, gather_camps::mining_camps())?;
        camp_table(scope, &plain_rows(camps))
    })
}

fn fishing_table<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, Value<'s>> {
    cached(scope, SLOT_FISHING, |scope| {
        let fishing = facts(scope, gather_camps::fishing_rows())?;
        let rows: Vec<RowView<'_>> = fishing
            .iter()
            .map(|row| RowView {
                camp: row.camp,
                keys: &row.keys,
                range: row.range,
                obstacles: row.obstacles,
                notes: row.notes.as_deref(),
            })
            .collect();
        camp_table(scope, &rows)
    })
}

fn surface_array<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, Value<'s>> {
    cached(scope, SLOT_COOK_SURFACES, |scope| {
        let surfaces = facts(scope, cook::fish_camp_cook_surfaces())?;
        let mut out = Vec::with_capacity(surfaces.len());
        for surface in surfaces {
            out.push(surface_object(scope, surface)?);
        }
        Ok(array(scope, &out))
    })
}

/// One curated cook surface object: `approach` first only where the literal wrote it first.
fn surface_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    surface: &FishCookSurface,
) -> JsResult<'s, Value<'s>> {
    let mut names: Vec<&'static str> = Vec::with_capacity(7);
    let mut values: Vec<Value<'s>> = Vec::with_capacity(7);
    if surface.approach_first {
        if let Some(approach) = surface.approach {
            names.push("approach");
            values.push(tile(scope, approach)?);
        }
    }
    names.push("stand");
    values.push(tile(scope, surface.stand)?);
    if let Some(loc) = surface.loc {
        names.push("loc");
        values.push(tile(scope, loc)?);
    }
    names.push("locName");
    values.push(cb::string(scope, surface.loc_name));
    names.push("kind");
    values.push(cb::string(scope, surface.kind.as_str()));
    if let Some(label) = surface.label {
        names.push("label");
        values.push(cb::string(scope, label));
    }
    if let Some(notes) = surface.notes {
        names.push("notes");
        values.push(cb::string(scope, notes));
    }
    object(scope, &names, &values)
}

/// The cached object of a surface by index (the one `CATHERBY_RANGE` shared by two roles stays one object).
fn surface_by_index<'s>(scope: &mut v8::HandleScope<'s>, index: usize) -> JsResult<'s, Value<'s>> {
    let surfaces = surface_array(scope)?;
    cb::get_index(scope, surfaces, index as u32)
}

fn methods_table<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, Value<'s>> {
    cached(scope, SLOT_METHODS, |scope| {
        let methods = facts(scope, fishing_gear::fishing_methods())?;
        let mut out = Vec::with_capacity(methods.len());
        for method in methods {
            let mut pieces = Vec::with_capacity(method.gear.len());
            for piece in method.gear {
                let values = [
                    cb::string(scope, piece.name),
                    int(scope, piece.min),
                    int(scope, piece.restock),
                ];
                pieces.push(object(scope, &["name", "min", "restock"], &values)?);
            }
            let values = [
                cb::string(scope, method.name),
                cb::string(scope, method.op),
                cb::string(scope, method.pair),
                array(scope, &pieces),
            ];
            out.push(object(scope, &["name", "op", "pair", "gear"], &values)?);
        }
        Ok(array(scope, &out))
    })
}

fn vendor_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    shop: FishingShop,
) -> JsResult<'s, Value<'s>> {
    let slot = match shop {
        FishingShop::Gerrant => SLOT_GERRANT,
        FishingShop::Harry => SLOT_HARRY,
    };
    cached(scope, slot, |scope| {
        let tables = facts(scope, fishing_gear::fishing_tables())?;
        let vendor: &FishingVendor = tables.vendor(shop);
        let values = [
            cb::string(scope, vendor.keeper),
            tile(scope, vendor.stand)?,
            tile(scope, vendor.bank_stand)?,
        ];
        object(scope, &["keeper", "stand", "bankStand"], &values)
    })
}

fn shilo_vendor<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, Value<'s>> {
    cached(scope, SLOT_SHILO, |scope| {
        let vendor = facts(scope, gather_camps::shilo_water_vendor())?;
        let values = [
            cb::string(scope, vendor.keeper),
            tile(scope, vendor.stand)?,
            cb::string(scope, vendor.item),
        ];
        object(scope, &["keeper", "stand", "item"], &values)
    })
}

fn mining_labels<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, Value<'s>> {
    let labels = facts(scope, gather_camps::mining_option_labels())?;
    let rows: Vec<(&'static str, String)> = labels.collect();
    let names: Vec<&'static str> = rows.iter().map(|(name, _)| *name).collect();
    let values: Vec<Value<'s>> = rows
        .iter()
        .map(|(_, label)| cb::string(scope, label))
        .collect();
    object(scope, &names, &values)
}

fn shop_costs<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, Value<'s>> {
    let costs = facts(scope, fishing_gear::fishing_shop_costs())?;
    let names: Vec<&'static str> = costs.iter().map(|(name, _)| *name).collect();
    let values: Vec<Value<'s>> = costs.iter().map(|(_, cost)| int(scope, *cost)).collect();
    object(scope, &names, &values)
}

fn set_of<'s>(scope: &mut v8::HandleScope<'s>, items: &[Value<'s>]) -> JsResult<'s, Value<'s>> {
    let set = v8::Set::new(scope);
    for item in items {
        cb::catching(scope, |s| set.add(s, *item))?;
    }
    Ok(set.into())
}

/// `WHIRLPOOL_IDS` from the selected content's NPC records; `undefined` (the shim then refuses) without them.
fn whirlpool_ids<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, Value<'s>> {
    let Some(ids) = crate::scene::supply_v2::selected_data()
        .and_then(|data| fishing_gear::whirlpool_ids(&data))
    else {
        return Ok(undefined(scope));
    };
    let items: Vec<Value<'s>> = ids.into_iter().map(|id| int(scope, id)).collect();
    set_of(scope, &items)
}

/// The constant table `op` as a build without local game data publishes it.
fn unavailable_table<'s>(scope: &mut v8::HandleScope<'s>, op: i32) -> Value<'s> {
    match op {
        SHILO_VENDOR | GERRANT_VENDOR | HARRY_VENDOR | WHIRLPOOL_IDS => undefined(scope),
        MINING_LABELS | SHOP_COSTS => v8::Object::new(scope).into(),
        GERRANT_ONLY => v8::Set::new(scope).into(),
        _ => array(scope, &[]),
    }
}

fn dispatch<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, Value<'s>> {
    let operation = args.get(0);
    if !operation.is_int32() {
        return Err(cb::not_impl(scope, "gather catalog member"));
    }
    let op = operation.int32_value(scope).unwrap_or(-1);
    if op < MINING_LABEL {
        remember_tile(scope, args.get(1))?;
        // A build without local game data loads every script that imports a catalog module:
        // the constant tables are then empty (or `undefined` for a single row), and only a
        // member that needs a table refuses.
        if gather_camps::mining_camps().is_err() {
            return Ok(unavailable_table(scope, op));
        }
    }
    match op {
        WOODCUTTING_TABLE => woodcutting_table(scope),
        MINING_TABLE => mining_table(scope),
        FISHING_TABLE => fishing_table(scope),
        SHILO_VENDOR => shilo_vendor(scope),
        METHODS_TABLE => methods_table(scope),
        GERRANT_VENDOR => vendor_object(scope, FishingShop::Gerrant),
        HARRY_VENDOR => vendor_object(scope, FishingShop::Harry),
        MINING_LABELS => mining_labels(scope),
        METHOD_OPTIONS => {
            let names = facts(scope, fishing_gear::method_names())?;
            Ok(strings(scope, names))
        }
        GEAR_NAMES => {
            let names = facts(scope, fishing_gear::all_gear_names())?;
            Ok(strings(scope, names))
        }
        WHIRLPOOL_IDS => whirlpool_ids(scope),
        SHOP_COSTS => shop_costs(scope),
        COOK_SURFACES => surface_array(scope),
        GERRANT_ONLY => {
            let only = facts(scope, fishing_gear::gerrant_only_fishing())?;
            let items: Vec<Value<'s>> = only.iter().map(|name| cb::string(scope, name)).collect();
            set_of(scope, &items)
        }
        MINING_LABEL => mining_label(scope, args.get(1)),
        RESOLVE_METHOD => resolve_method(scope, args.get(1)),
        KEEP_NAMES => keep_names(scope, args.get(1)),
        GEAR_LABEL => gear_label(scope, args.get(1)),
        HAS_GEAR => has_gear(scope, args.get(1), args.get(2)),
        MISSING_GEAR => missing_gear(scope, args.get(1), args.get(2)),
        RESTOCK_PLAN => restock_plan(scope, args.get(1), args.get(2), args.get(3)),
        SPOT_MATCHES => spot_matches(scope, args.get(1), args.get(2)),
        SHOP_COST => shop_cost(scope, args.get(1)),
        VENDOR_FOR => vendor_for(scope, args.get(1), args.get(2)),
        IS_BAIT => is_bait(scope, args.get(1)),
        WITH_BAIT => with_bait(scope, args.get(1), args.get(2)),
        PLAN_BUYS => plan_buys(scope, args.get(1), args.get(2), args.get(3), PlanShape::All),
        PLAN_ACQUIRE => plan_buys(
            scope,
            args.get(1),
            args.get(2),
            args.get(3),
            PlanShape::First,
        ),
        PLANS_COST => plans_cost(scope, args.get(1)),
        SHOP_CART => plan_buys(
            scope,
            args.get(1),
            args.get(2),
            args.get(3),
            PlanShape::Cart,
        ),
        SHOPABLE_MISSING => shopable_missing(scope, args.get(1), args.get(2)),
        COOK_SURFACE_FOR => cook_surface_for_camp(scope, args.get(1), args.get(2)),
        RESOLVE_COOK => {
            resolve_cook_surface(scope, args.get(1), args.get(2), args.get(3), args.get(4))
        }
        _ => Err(cb::not_impl(scope, "gather catalog member")),
    }
}

/// Frozen `miningLocationLabel` (`miningLocations.ts:55-59`): the row's own `name` value, or the formatted label
/// when `recommendedCombat != null`.
fn mining_label<'s>(scope: &mut v8::HandleScope<'s>, loc: Value<'s>) -> JsResult<'s, Value<'s>> {
    let recommended = cb::get(scope, loc, "recommendedCombat")?;
    if recommended.is_null_or_undefined() {
        return cb::get(scope, loc, "name");
    }
    let name = cb::get(scope, loc, "name")?;
    let name = cb::to_string(scope, name)?;
    let recommended = cb::get(scope, loc, "recommendedCombat")?;
    let recommended = cb::to_string(scope, recommended)?;
    Ok(cb::string(
        scope,
        &format!("{name} ({recommended} Combat recommended)"),
    ))
}

/// Frozen `resolveFishMethod` (`fishingMethods.ts:90-92`): the exact-name method, else the first, as the one
/// materialized method object.
fn resolve_method<'s>(scope: &mut v8::HandleScope<'s>, name: Value<'s>) -> JsResult<'s, Value<'s>> {
    let methods = methods_table(scope)?;
    let index = match name.is_string() {
        true => {
            let wanted = cb::to_string(scope, name)?;
            facts(scope, fishing_gear::fishing_methods())?
                .iter()
                .position(|method| method.name == wanted)
                .unwrap_or(0)
        }
        false => 0,
    };
    cb::get_index(scope, methods, index as u32)
}

/// How the caller's count functions are reached.
enum Counts<'s> {
    /// `count(name)`, a plain call.
    Plain(Value<'s>),
    /// `w.heldCount(name)`, `w.invCount(name)`, `w.bankCount(name)`, methods of the world.
    World(Value<'s>),
    /// `invCount(name)` and `bankCount(name)`, plain calls.
    Pair(Value<'s>, Value<'s>),
}

/// The caller's gear array and count functions, read the way the frozen loops read them.
struct V8Gear<'a, 's> {
    scope: &'a mut v8::HandleScope<'s>,
    pieces: Vec<Value<'s>>,
    names: Vec<Option<Value<'s>>>,
    counts: Counts<'s>,
    opts: Option<Value<'s>>,
}

impl<'a, 's> V8Gear<'a, 's> {
    /// `gear` is the caller's array; `iterate` names the array method the frozen code calls on it (empty for a
    /// `for...of`), for the `TypeError` a non-array earns.
    fn open(
        scope: &'a mut v8::HandleScope<'s>,
        gear: Value<'s>,
        iterate: &str,
        counts: Counts<'s>,
        opts: Option<Value<'s>>,
    ) -> JsResult<'s, Self> {
        if gear.is_null_or_undefined() {
            let seen = if gear.is_null() { "null" } else { "undefined" };
            let message = if iterate.is_empty() {
                format!("method.gear is not iterable (found {seen})")
            } else {
                format!("Cannot read properties of {seen} (reading '{iterate}')")
            };
            return Err(cb::type_error(scope, &message));
        }
        if !gear.is_array() {
            let message = if iterate.is_empty() {
                "method.gear is not iterable".to_string()
            } else {
                format!("method.gear.{iterate} is not a function")
            };
            return Err(cb::type_error(scope, &message));
        }
        let length = cb::array_length(scope, gear)?;
        let mut pieces = Vec::with_capacity(length as usize);
        for index in 0..length {
            pieces.push(cb::get_index(scope, gear, index)?);
        }
        let names = vec![None; pieces.len()];
        Ok(Self {
            scope,
            pieces,
            names,
            counts,
            opts,
        })
    }

    /// `g.name`, read once and kept: the value the callbacks receive and the plans return.
    fn name_value(&mut self, piece: usize) -> JsResult<'s, Value<'s>> {
        if let Some(name) = self.names[piece] {
            return Ok(name);
        }
        let name = cb::get(self.scope, self.pieces[piece], "name")?;
        self.names[piece] = Some(name);
        Ok(name)
    }
}

impl<'s> GearHost for V8Gear<'_, 's> {
    type Error = cb::Throw<'s>;

    fn facts(&mut self, error: FactError) -> cb::Throw<'s> {
        cb::type_error(self.scope, &format!("gather catalog: {error:?}"))
    }

    fn pieces(&self) -> usize {
        self.pieces.len()
    }

    fn min(&mut self, piece: usize) -> JsResult<'s, f64> {
        let min = cb::get(self.scope, self.pieces[piece], "min")?;
        cb::number(self.scope, min)
    }

    fn restock(&mut self, piece: usize) -> JsResult<'s, f64> {
        let restock = cb::get(self.scope, self.pieces[piece], "restock")?;
        cb::number(self.scope, restock)
    }

    fn lowered(&mut self, piece: usize) -> JsResult<'s, String> {
        let name = self.name_value(piece)?;
        lowercase(self.scope, name)
    }

    fn count(&mut self, which: Count, item: Item) -> JsResult<'s, f64> {
        let argument = match item {
            Item::Piece(piece) => self.name_value(piece)?,
            Item::Coins => cb::string(self.scope, "Coins"),
        };
        let nothing = undefined(self.scope);
        let result = match (&self.counts, which) {
            (Counts::Plain(count), _) => {
                cb::call(self.scope, *count, nothing, &[argument], "count")?
            }
            (Counts::World(world), Count::Held) => {
                cb::call_method(self.scope, *world, "heldCount", &[argument], "w.heldCount")?
            }
            (Counts::World(world), Count::Inv) => {
                cb::call_method(self.scope, *world, "invCount", &[argument], "w.invCount")?
            }
            (Counts::World(world), _) => {
                cb::call_method(self.scope, *world, "bankCount", &[argument], "w.bankCount")?
            }
            (Counts::Pair(inv, _), Count::Inv) => {
                cb::call(self.scope, *inv, nothing, &[argument], "invCount")?
            }
            (Counts::Pair(_, bank), _) => {
                cb::call(self.scope, *bank, nothing, &[argument], "bankCount")?
            }
        };
        cb::number(self.scope, result)
    }

    fn near(
        &mut self,
        harry: &FishingVendor,
        gerrant: &FishingVendor,
    ) -> JsResult<'s, Option<(f64, f64)>> {
        let near = match self.opts {
            Some(opts) => cb::get(self.scope, opts, "near")?,
            None => undefined(self.scope),
        };
        near_distances(self.scope, near, harry, gerrant)
    }
}

/// `value.toLowerCase()` as text; a value without the method earns the engine's `TypeError`.
fn lowercase<'s>(scope: &mut v8::HandleScope<'s>, value: Value<'s>) -> JsResult<'s, String> {
    let lowered = cb::call_method(scope, value, "toLowerCase", &[], "name.toLowerCase")?;
    cb::to_string(scope, lowered)
}

/// `Math.hypot(near.x - stand.x, near.z - stand.z)` for Harry then Gerrant, or `None` when `near` is falsy.
fn near_distances<'s>(
    scope: &mut v8::HandleScope<'s>,
    near: Value<'s>,
    harry: &FishingVendor,
    gerrant: &FishingVendor,
) -> JsResult<'s, Option<(f64, f64)>> {
    if !cb::truthy(scope, near) {
        return Ok(None);
    }
    let distance = |scope: &mut v8::HandleScope<'s>, stand: WorldTile| -> JsResult<'s, f64> {
        let x = cb::get(scope, near, "x")?;
        let stand_x = int(scope, stand.x);
        let dx = cb::sub(scope, x, stand_x)?;
        let z = cb::get(scope, near, "z")?;
        let stand_z = int(scope, stand.z);
        let dz = cb::sub(scope, z, stand_z)?;
        let dx = cb::number(scope, dx)?;
        let dz = cb::number(scope, dz)?;
        Ok(dx.hypot(dz))
    };
    let to_harry = distance(scope, harry.stand)?;
    let to_gerrant = distance(scope, gerrant.stand)?;
    Ok(Some((to_harry, to_gerrant)))
}

fn method_gear<'s>(scope: &mut v8::HandleScope<'s>, method: Value<'s>) -> JsResult<'s, Value<'s>> {
    cb::get(scope, method, "gear")
}

fn keep_names<'s>(scope: &mut v8::HandleScope<'s>, method: Value<'s>) -> JsResult<'s, Value<'s>> {
    let gear = method_gear(scope, method)?;
    let mut host = V8Gear::open(scope, gear, "map", Counts::Plain(gear), None)?;
    let mut names = Vec::with_capacity(host.pieces());
    for piece in 0..host.pieces() {
        names.push(host.name_value(piece)?);
    }
    Ok(array(host.scope, &names))
}

/// `method.gear.map(g => g.name).join(' + ')`: a nullish name joins as the empty string.
fn gear_label<'s>(scope: &mut v8::HandleScope<'s>, method: Value<'s>) -> JsResult<'s, Value<'s>> {
    let gear = method_gear(scope, method)?;
    let mut host = V8Gear::open(scope, gear, "map", Counts::Plain(gear), None)?;
    let mut parts = Vec::with_capacity(host.pieces());
    for piece in 0..host.pieces() {
        let name = host.name_value(piece)?;
        parts.push(if name.is_null_or_undefined() {
            String::new()
        } else {
            cb::to_string(host.scope, name)?
        });
    }
    Ok(cb::string(host.scope, &parts.join(" + ")))
}

fn has_gear<'s>(
    scope: &mut v8::HandleScope<'s>,
    method: Value<'s>,
    count: Value<'s>,
) -> JsResult<'s, Value<'s>> {
    let gear = method_gear(scope, method)?;
    let mut host = V8Gear::open(scope, gear, "every", Counts::Plain(count), None)?;
    let held = gear::has_gear(&mut host)?;
    Ok(boolean(host.scope, held))
}

fn missing_gear<'s>(
    scope: &mut v8::HandleScope<'s>,
    method: Value<'s>,
    count: Value<'s>,
) -> JsResult<'s, Value<'s>> {
    let gear = method_gear(scope, method)?;
    let mut host = V8Gear::open(scope, gear, "filter", Counts::Plain(count), None)?;
    let missing = gear::missing_gear(&mut host)?;
    let pieces: Vec<Value<'s>> = missing
        .into_iter()
        .map(|piece| host.pieces[piece])
        .collect();
    Ok(array(host.scope, &pieces))
}

fn shopable_missing<'s>(
    scope: &mut v8::HandleScope<'s>,
    pieces: Value<'s>,
    count: Value<'s>,
) -> JsResult<'s, Value<'s>> {
    let mut host = V8Gear::open(scope, pieces, "filter", Counts::Plain(count), None)?;
    let missing = gear::shopable_missing_gear(&mut host)?;
    let mut names = Vec::with_capacity(missing.len());
    for piece in missing {
        names.push(host.name_value(piece)?);
    }
    Ok(array(host.scope, &names))
}

fn restock_plan<'s>(
    scope: &mut v8::HandleScope<'s>,
    method: Value<'s>,
    inv: Value<'s>,
    bank: Value<'s>,
) -> JsResult<'s, Value<'s>> {
    let gear = method_gear(scope, method)?;
    let mut host = V8Gear::open(scope, gear, "", Counts::Pair(inv, bank), None)?;
    let plan = gear::restock_plan(&mut host)?;
    let mut rows = Vec::with_capacity(plan.len());
    for (piece, qty) in plan {
        let name = host.name_value(piece)?;
        let qty = cb::num(host.scope, qty);
        rows.push(object(host.scope, &["name", "qty"], &[name, qty])?);
    }
    Ok(array(host.scope, &rows))
}

/// Frozen `spotMatchesMethod` (`fishingMethods.ts:137-149`): lowercase every action, then the method's actions.
fn spot_matches<'s>(
    scope: &mut v8::HandleScope<'s>,
    actions: Value<'s>,
    method: Value<'s>,
) -> JsResult<'s, Value<'s>> {
    let length = cb::array_length(scope, actions)?;
    let mut ops: Vec<Value<'s>> = Vec::with_capacity(length as usize);
    for index in 0..length {
        if !cb::has_index(scope, actions, index)? {
            continue;
        }
        let action = cb::get_index(scope, actions, index)?;
        ops.push(cb::call_method(
            scope,
            action,
            "toLowerCase",
            &[],
            "a.toLowerCase",
        )?);
    }
    let op = cb::get(scope, method, "op")?;
    let primary = cb::call_method(scope, op, "toLowerCase", &[], "method.op.toLowerCase")?;
    let contains = |ops: &[Value<'s>], wanted: Value<'s>| {
        ops.iter().any(|candidate| candidate.strict_equals(wanted))
    };
    if !contains(&ops, primary) {
        return Ok(boolean(scope, false));
    }
    let pair = cb::get(scope, method, "pair")?;
    let trimmed = cb::call_method(scope, pair, "trim", &[], "method.pair.trim")?;
    let pair = cb::call_method(
        scope,
        trimmed,
        "toLowerCase",
        &[],
        "method.pair.trim().toLowerCase",
    )?;
    let unpaired = !cb::truthy(scope, pair) || pair.strict_equals(primary);
    let matches = unpaired || contains(&ops, pair);
    Ok(boolean(scope, matches))
}

fn shop_cost<'s>(scope: &mut v8::HandleScope<'s>, name: Value<'s>) -> JsResult<'s, Value<'s>> {
    let lowered = lowercase(scope, name)?;
    let cost = facts(scope, fishing_gear::shop_cost(&lowered))?;
    Ok(match cost {
        Some(cost) => int(scope, cost),
        None => v8::null(scope).into(),
    })
}

fn vendor_for<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: Value<'s>,
    near: Value<'s>,
) -> JsResult<'s, Value<'s>> {
    let lowered = lowercase(scope, name)?;
    let tables = facts(scope, fishing_gear::fishing_tables())?;
    let shop = tables.vendor_for(&lowered, |harry, gerrant| {
        near_distances(scope, near, harry, gerrant)
    })?;
    vendor_object(scope, shop)
}

fn is_bait<'s>(scope: &mut v8::HandleScope<'s>, piece: Value<'s>) -> JsResult<'s, Value<'s>> {
    let restock = cb::get(scope, piece, "restock")?;
    let restock = cb::number(scope, restock)?;
    let bait = fishing_gear::is_bait_piece(restock, || {
        let name = cb::get(scope, piece, "name")?;
        lowercase(scope, name)
    })?;
    Ok(boolean(scope, bait))
}

/// Frozen `withBaitTarget`: `{ gear }` where a bait piece is a copy of the caller's piece with `restock` set to
/// the target and every other piece is the caller's own object.
fn with_bait<'s>(
    scope: &mut v8::HandleScope<'s>,
    method: Value<'s>,
    bait_qty: Value<'s>,
) -> JsResult<'s, Value<'s>> {
    let floored = {
        let value = cb::number(scope, bait_qty)?;
        value.floor()
    };
    let target = js_max(1.0, floored);
    let gear_value = method_gear(scope, method)?;
    let nothing = undefined(scope);
    let mut host = V8Gear::open(scope, gear_value, "map", Counts::Plain(nothing), None)?;
    let (_, flags) = gear::bait_target(&mut host, floored)?;
    let mut out = Vec::with_capacity(flags.len());
    for (piece, bait) in flags.into_iter().enumerate() {
        out.push(if bait {
            copy_with_restock(host.scope, host.pieces[piece], target)?
        } else {
            host.pieces[piece]
        });
    }
    let gear_array = array(host.scope, &out);
    object(host.scope, &["gear"], &[gear_array])
}

/// The own keys of a method's gear piece (`api::fishing_gear`), the one piece shape the catalog knows.
const PIECE_KEYS: [&str; 3] = ["name", "min", "restock"];

/// `{ ...piece, restock: target }`. A piece with exactly the catalog's own keys, in order, becomes an instance of
/// the cached piece shape. Any other piece, with whatever string, index or symbol keys the caller gave it
/// (including an own `__proto__`, which stays an own data property), is copied by spread semantics onto a plain
/// object: nothing about a caller's key list is cached, so callers cannot grow the catalog's per-isolate state.
fn copy_with_restock<'s>(
    scope: &mut v8::HandleScope<'s>,
    piece: Value<'s>,
    target: f64,
) -> JsResult<'s, Value<'s>> {
    let Ok(source) = v8::Local::<v8::Object>::try_from(piece) else {
        return Err(cb::not_impl(scope, "bait piece that is not an object"));
    };
    let mut options = v8::GetPropertyNamesArgsBuilder::new();
    options
        .mode(v8::KeyCollectionMode::OwnOnly)
        .property_filter(v8::PropertyFilter::ONLY_ENUMERABLE)
        .index_filter(v8::IndexFilter::IncludeIndices)
        .key_conversion(v8::KeyConversionMode::ConvertToString);
    let listed = cb::catching(scope, |s| source.get_own_property_names(s, options.build()))?;
    let length = listed.length();
    let mut names: Vec<v8::Local<'s, v8::Name>> = Vec::with_capacity(length as usize);
    let mut known = length as usize == PIECE_KEYS.len();
    for index in 0..length {
        let name = cb::get_index(scope, listed.into(), index)?;
        if known {
            let expected = key(scope, PIECE_KEYS[index as usize]);
            known = name.strict_equals(expected.into());
        }
        let Ok(name) = v8::Local::<v8::Name>::try_from(name) else {
            return Err(cb::type_error(
                scope,
                "gather catalog: a property key is neither a string nor a symbol",
            ));
        };
        names.push(name);
    }
    let mut values: Vec<Value<'s>> = Vec::with_capacity(names.len());
    for name in &names {
        values.push(cb::catching(scope, |s| source.get(s, (*name).into()))?);
    }
    let target = cb::num(scope, target);
    if known {
        values[2] = target;
        return object(scope, &PIECE_KEYS, &values);
    }
    let copy = v8::Object::new(scope);
    let restock = key(scope, "restock");
    let named = names
        .iter()
        .zip(&values)
        .map(|(name, value)| (*name, *value))
        .chain(std::iter::once((restock.into(), target)));
    for (name, value) in named {
        if !cb::catching(scope, |s| copy.create_data_property(s, name, value))? {
            return Err(cb::type_error(
                scope,
                "gather catalog: could not set a property",
            ));
        }
    }
    Ok(copy.into())
}

/// Which plans a `planFishingGearBuys` family member returns.
#[derive(Clone, Copy)]
enum PlanShape {
    /// `planFishingGearBuys`: every plan.
    All,
    /// `planFishingGearAcquire`: the first plan or `null`.
    First,
    /// `fishingGearShopCart`: the plans of the first plan's shop.
    Cart,
}

fn plan_buys<'s>(
    scope: &mut v8::HandleScope<'s>,
    method: Value<'s>,
    world: Value<'s>,
    opts: Value<'s>,
    shape: PlanShape,
) -> JsResult<'s, Value<'s>> {
    let (opts, bait_qty) = if opts.is_undefined() {
        (None, 1000.0)
    } else {
        let bait = cb::get(scope, opts, "baitQty")?;
        let bait = if bait.is_null_or_undefined() {
            cb::num(scope, 1000.0)
        } else {
            bait
        };
        let bait = cb::number(scope, bait)?;
        (Some(opts), bait)
    };
    let gear_value = method_gear(scope, method)?;
    let mut host = V8Gear::open(scope, gear_value, "", Counts::World(world), opts)?;
    let plans = gear::buy_plans(&mut host, bait_qty)?;
    let plans = match shape {
        PlanShape::Cart => gear::shop_cart(plans),
        PlanShape::First => plans.into_iter().take(1).collect(),
        PlanShape::All => plans,
    };
    if matches!(shape, PlanShape::First) {
        return match plans.first() {
            Some(plan) => plan_object(&mut host, plan),
            None => Ok(v8::null(host.scope).into()),
        };
    }
    let mut rows = Vec::with_capacity(plans.len());
    for plan in &plans {
        rows.push(plan_object(&mut host, plan)?);
    }
    Ok(array(host.scope, &rows))
}

/// `{ kind: 'buy', name, cost, qty, vendor, equip: false, reason }`.
fn plan_object<'s>(host: &mut V8Gear<'_, 's>, plan: &BuyPlan) -> JsResult<'s, Value<'s>> {
    let name = host.name_value(plan.piece)?;
    let scope = &mut *host.scope;
    let vendor = vendor_object(scope, plan.shop)?;
    let qty = cb::num(scope, plan.qty);
    let shown_qty = cb::to_string(scope, qty)?;
    let shown_name = cb::to_string(scope, name)?;
    let values = [
        cb::string(scope, "buy"),
        name,
        cb::num(scope, plan.cost),
        qty,
        vendor,
        boolean(scope, false),
        cb::string(scope, &format!("buy {shown_qty}\u{d7} {shown_name}")),
    ];
    object(
        scope,
        &["kind", "name", "cost", "qty", "vendor", "equip", "reason"],
        &values,
    )
}

/// Frozen `buyPlansCost`: `plans.reduce((sum, p) => sum + p.cost, 0)` over the caller's array, skipping holes.
fn plans_cost<'s>(scope: &mut v8::HandleScope<'s>, plans: Value<'s>) -> JsResult<'s, Value<'s>> {
    if !plans.is_array() {
        let message = if plans.is_null_or_undefined() {
            "Cannot read properties of undefined (reading 'reduce')"
        } else {
            "plans.reduce is not a function"
        };
        return Err(cb::type_error(scope, message));
    }
    let length = cb::array_length(scope, plans)?;
    let mut sum = cb::num(scope, 0.0);
    for index in 0..length {
        if !cb::has_index(scope, plans, index)? {
            continue;
        }
        let plan = cb::get_index(scope, plans, index)?;
        let cost = cb::get(scope, plan, "cost")?;
        sum = cb::add(scope, sum, cost)?;
    }
    Ok(sum)
}

/// The property key `FISH_CAMP_COOK_PLANS[campName]` reads. A symbol key names no camp.
fn plan_key<'s>(scope: &mut v8::HandleScope<'s>, camp: Value<'s>) -> JsResult<'s, Option<String>> {
    if camp.is_symbol() {
        return Ok(None);
    }
    cb::to_string(scope, camp).map(Some)
}

fn role_of<'s>(scope: &mut v8::HandleScope<'s>, role: Value<'s>) -> CookRole {
    let bank = cb::string(scope, "bank");
    if role.strict_equals(bank) {
        CookRole::Bank
    } else {
        CookRole::Pier
    }
}

/// Frozen `cookSurfaceForFishCamp` (`cookingRanges.ts:169-181`): the camp's curated surface for the role, as the
/// one cached object, or `null`.
fn cook_surface_for_camp<'s>(
    scope: &mut v8::HandleScope<'s>,
    camp: Value<'s>,
    role: Value<'s>,
) -> JsResult<'s, Value<'s>> {
    let name = plan_key(scope, camp)?;
    let role = role_of(scope, role);
    let index = match name {
        Some(name) => facts(scope, cook::fish_camp_cook_surface_index(&name, role))?,
        None => None,
    };
    match index {
        Some(index) => surface_by_index(scope, index),
        None => Ok(v8::null(scope).into()),
    }
}

/// Frozen `resolveFishCampCookSurface` (`cookingRanges.ts:184-197`): the curated surface when the camp name is
/// truthy and has one, else the nearest selected `Range`.
fn resolve_cook_surface<'s>(
    scope: &mut v8::HandleScope<'s>,
    camp: Value<'s>,
    spot: Value<'s>,
    max_cheb: Value<'s>,
    role: Value<'s>,
) -> JsResult<'s, Value<'s>> {
    if cb::truthy(scope, camp) {
        let curated = cook_surface_for_camp(scope, camp, role)?;
        if !curated.is_null() {
            return Ok(curated);
        }
    }
    nearest_cooking_range(scope, spot, max_cheb)
}

/// Frozen `nearestCookingRange` (`cookingRanges.ts:50-71`) over the selected `Range` placements: the nearest on
/// the origin's plane within `maxCheb` (first wins a tie), as a fresh surface object.
fn nearest_cooking_range<'s>(
    scope: &mut v8::HandleScope<'s>,
    origin: Value<'s>,
    max_cheb: Value<'s>,
) -> JsResult<'s, Value<'s>> {
    let level = {
        let level = cb::get(scope, origin, "level")?;
        if level.is_null_or_undefined() {
            int(scope, 0)
        } else {
            level
        }
    };
    let max_cheb = cb::number(scope, max_cheb)?;
    let Some(data) = crate::scene::supply_v2::selected_data() else {
        return Ok(v8::null(scope).into());
    };
    let cook_surfaces = facts(scope, data.cook_surfaces())?;
    let found = cook_pairing::nearest_range(cook_surfaces, max_cheb, |surface| {
        let plane = int(scope, surface.level);
        if !level.strict_equals(plane) {
            return Ok(None);
        }
        let origin_x = cb::get(scope, origin, "x")?;
        let loc_x = int(scope, surface.x);
        let dx = cb::sub(scope, origin_x, loc_x)?;
        let origin_z = cb::get(scope, origin, "z")?;
        let loc_z = int(scope, surface.z);
        let dz = cb::sub(scope, origin_z, loc_z)?;
        let dx = cb::number(scope, dx)?.abs();
        let dz = cb::number(scope, dz)?.abs();
        Ok(Some(js_max(dx, dz)))
    })?;
    let Some((surface, distance)) = found else {
        return Ok(v8::null(scope).into());
    };
    let shown = {
        let value = cb::num(scope, distance);
        cb::to_string(scope, value)?
    };
    let values = [
        tile(
            scope,
            WorldTile {
                x: surface.x,
                z: surface.z - 1,
                level: surface.level,
            },
        )?,
        tile(scope, surface.tile())?,
        cb::string(scope, "Range"),
        cb::string(scope, "range"),
        cb::string(scope, &format!("Range @ {},{}", surface.x, surface.z)),
        cb::string(scope, &format!("cheb {shown} from origin")),
    ];
    object(
        scope,
        &["stand", "loc", "locName", "kind", "label", "notes"],
        &values,
    )
}

#[cfg(test)]
#[path = "gather_catalog_v8_tests.rs"]
mod tests;
