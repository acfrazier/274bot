//! Bounded catalog/air queries. Foreign predicates are called only by the
//! Rust loop; no route search or selection policy runs in JavaScript.
use super::callback_v8;
use crate::bank_select::{self, FromTile};
use crate::cook_locations::{self, CookLocation};
use api::snapshot::WorldTile;
use rustyscript::deno_core::serde_v8;
use rustyscript::Runtime;
use serde::Deserialize;
use serde_json::json;
use std::{cell::RefCell, rc::Rc};

const BANK_ACCESS_SHAPES: usize = 5;
const BANK_REQUIREMENT_SHAPES: usize = 8;
const BANK_RESULT_SHAPE_COUNT: usize = 2 * BANK_ACCESS_SHAPES * BANK_REQUIREMENT_SHAPES;

/// Per-isolate V8 keys and immutable result templates. This cache holds no
/// materialized bank rows. Session reset can keep these bounded shapes, and
/// stopping the isolate drops every Global on its owning thread.
struct BankResultShapes {
    keys: BankResultKeys,
    shapes: [Option<BankResultShape>; BANK_RESULT_SHAPE_COUNT],
}

struct BankResultKeys {
    name: v8::Global<v8::String>,
    tile: v8::Global<v8::String>,
    approach: v8::Global<v8::String>,
    npc_access: v8::Global<v8::String>,
    access: v8::Global<v8::String>,
    op: v8::Global<v8::String>,
    open_first: v8::Global<v8::String>,
    choose: v8::Global<v8::String>,
    requires: v8::Global<v8::String>,
    skill: v8::Global<v8::String>,
    quest: v8::Global<v8::String>,
    setting: v8::Global<v8::String>,
    level: v8::Global<v8::String>,
}

struct BankResultShape {
    row: v8::Global<v8::ObjectTemplate>,
    access: Option<v8::Global<v8::ObjectTemplate>>,
    open_first: Option<v8::Global<v8::ObjectTemplate>>,
    requires: Option<v8::Global<v8::ObjectTemplate>>,
    skill: Option<v8::Global<v8::ObjectTemplate>>,
}

struct BankResultKeyLocals<'s> {
    name: v8::Local<'s, v8::String>,
    tile: v8::Local<'s, v8::String>,
    approach: v8::Local<'s, v8::String>,
    npc_access: v8::Local<'s, v8::String>,
    access: v8::Local<'s, v8::String>,
    op: v8::Local<'s, v8::String>,
    open_first: v8::Local<'s, v8::String>,
    choose: v8::Local<'s, v8::String>,
    requires: v8::Local<'s, v8::String>,
    skill: v8::Local<'s, v8::String>,
    quest: v8::Local<'s, v8::String>,
    setting: v8::Local<'s, v8::String>,
    level: v8::Local<'s, v8::String>,
}

struct BankResultShapeLocals<'s> {
    row: v8::Local<'s, v8::ObjectTemplate>,
    access: Option<v8::Local<'s, v8::ObjectTemplate>>,
    open_first: Option<v8::Local<'s, v8::ObjectTemplate>>,
    requires: Option<v8::Local<'s, v8::ObjectTemplate>>,
    skill: Option<v8::Local<'s, v8::ObjectTemplate>>,
}

impl BankResultKeys {
    fn create(scope: &mut v8::HandleScope) -> Result<Self, String> {
        fn key(
            scope: &mut v8::HandleScope,
            name: &'static str,
        ) -> Result<v8::Global<v8::String>, String> {
            let value =
                v8::String::new_from_utf8(scope, name.as_bytes(), v8::NewStringType::Internalized)
                    .ok_or_else(|| {
                        format!("bank result key {name}: v8 string allocation failed")
                    })?;
            Ok(v8::Global::new(scope, value))
        }

        Ok(Self {
            name: key(scope, "name")?,
            tile: key(scope, "tile")?,
            approach: key(scope, "approach")?,
            npc_access: key(scope, "npcAccess")?,
            access: key(scope, "access")?,
            op: key(scope, "op")?,
            open_first: key(scope, "openFirst")?,
            choose: key(scope, "choose")?,
            requires: key(scope, "requires")?,
            skill: key(scope, "skill")?,
            quest: key(scope, "quest")?,
            setting: key(scope, "setting")?,
            level: key(scope, "level")?,
        })
    }

    fn local<'s>(
        &self,
        scope: &mut v8::HandleScope<'s>,
        field: &str,
    ) -> Option<v8::Local<'s, v8::String>> {
        let key = match field {
            "name" => &self.name,
            "tile" => &self.tile,
            "approach" => &self.approach,
            "npcAccess" => &self.npc_access,
            "access" => &self.access,
            "op" => &self.op,
            "openFirst" => &self.open_first,
            "choose" => &self.choose,
            "requires" => &self.requires,
            "skill" => &self.skill,
            "quest" => &self.quest,
            "setting" => &self.setting,
            "level" => &self.level,
            _ => return None,
        };
        Some(v8::Local::new(scope, key))
    }

    fn locals<'s>(&self, scope: &mut v8::HandleScope<'s>) -> BankResultKeyLocals<'s> {
        BankResultKeyLocals {
            name: v8::Local::new(scope, &self.name),
            tile: v8::Local::new(scope, &self.tile),
            approach: v8::Local::new(scope, &self.approach),
            npc_access: v8::Local::new(scope, &self.npc_access),
            access: v8::Local::new(scope, &self.access),
            op: v8::Local::new(scope, &self.op),
            open_first: v8::Local::new(scope, &self.open_first),
            choose: v8::Local::new(scope, &self.choose),
            requires: v8::Local::new(scope, &self.requires),
            skill: v8::Local::new(scope, &self.skill),
            quest: v8::Local::new(scope, &self.quest),
            setting: v8::Local::new(scope, &self.setting),
            level: v8::Local::new(scope, &self.level),
        }
    }
}

impl BankResultShape {
    fn create(
        scope: &mut v8::HandleScope,
        keys: &BankResultKeys,
        value: &serde_json::Value,
    ) -> Result<Self, String> {
        let row = bank_result_template(scope, value, keys)?;
        let access_value = value.get("npcAccess").or_else(|| value.get("access"));
        let access = access_value
            .map(|value| bank_result_template(scope, value, keys))
            .transpose()?;
        let open_first = value
            .get("access")
            .and_then(|value| value.get("openFirst"))
            .map(|value| bank_result_template(scope, value, keys))
            .transpose()?;
        let requires = value
            .get("requires")
            .map(|value| bank_result_template(scope, value, keys))
            .transpose()?;
        let skill = value
            .get("requires")
            .and_then(|value| value.get("skill"))
            .map(|value| bank_result_template(scope, value, keys))
            .transpose()?;
        Ok(Self {
            row,
            access,
            open_first,
            requires,
            skill,
        })
    }

    fn locals<'s>(&self, scope: &mut v8::HandleScope<'s>) -> BankResultShapeLocals<'s> {
        BankResultShapeLocals {
            row: v8::Local::new(scope, &self.row),
            access: self
                .access
                .as_ref()
                .map(|template| v8::Local::new(scope, template)),
            open_first: self
                .open_first
                .as_ref()
                .map(|template| v8::Local::new(scope, template)),
            requires: self
                .requires
                .as_ref()
                .map(|template| v8::Local::new(scope, template)),
            skill: self
                .skill
                .as_ref()
                .map(|template| v8::Local::new(scope, template)),
        }
    }
}

impl BankResultShapes {
    fn create(scope: &mut v8::HandleScope) -> Result<Self, String> {
        Ok(Self {
            keys: BankResultKeys::create(scope)?,
            shapes: std::array::from_fn(|_| None),
        })
    }

    fn shape<'s>(
        &mut self,
        scope: &mut v8::HandleScope<'s>,
        value: &serde_json::Value,
        index: usize,
    ) -> Result<(BankResultShapeLocals<'s>, BankResultKeyLocals<'s>), String> {
        let Some(slot) = self.shapes.get_mut(index) else {
            return Err("bank result shape key exceeds its schema bound".into());
        };
        if slot.is_none() {
            *slot = Some(BankResultShape::create(scope, &self.keys, value)?);
        }
        let shape = slot
            .as_ref()
            .ok_or_else(|| "bank result shape was not cached".to_string())?;
        Ok((shape.locals(scope), self.keys.locals(scope)))
    }
}

fn bank_result_template(
    scope: &mut v8::HandleScope,
    value: &serde_json::Value,
    keys: &BankResultKeys,
) -> Result<v8::Global<v8::ObjectTemplate>, String> {
    let fields = value
        .as_object()
        .ok_or_else(|| "bank result shape is not an object".to_string())?;
    let template = v8::ObjectTemplate::new(scope);
    let placeholder: v8::Local<v8::Data> = v8::undefined(scope).into();
    for field in fields.keys() {
        let key = keys
            .local(scope, field)
            .ok_or_else(|| format!("bank result schema has no field `{field}`"))?;
        template.set(key.into(), placeholder);
    }
    Ok(v8::Global::new(scope, template))
}

/// Return the index of a schema shape. The product of two approach forms,
/// five access forms, and eight requirement forms bounds this cache at 80.
fn bank_result_shape_index(value: &serde_json::Value) -> Result<usize, String> {
    let fields = value
        .as_object()
        .ok_or_else(|| "bank result is not an object".to_string())?;
    let approach = if fields.contains_key("approach") {
        1
    } else {
        0
    };
    let access_shape = match (fields.get("npcAccess"), fields.get("access")) {
        (None, None) => 0,
        (Some(access), None) => {
            let access = access
                .as_object()
                .ok_or_else(|| "bank result npcAccess is not an object".to_string())?;
            if access.contains_key("choose") {
                2
            } else {
                1
            }
        }
        (None, Some(access)) => {
            let access = access
                .as_object()
                .ok_or_else(|| "bank result access is not an object".to_string())?;
            if access.contains_key("openFirst") {
                4
            } else {
                3
            }
        }
        (Some(_), Some(_)) => {
            return Err("bank result has both npcAccess and access".into());
        }
    };
    let requirement_shape = match fields.get("requires") {
        None => 0,
        Some(value) => {
            let requirements = value
                .as_object()
                .ok_or_else(|| "bank result requires is not an object".to_string())?;
            let skill = if requirements.contains_key("skill") {
                1_usize
            } else {
                0
            };
            let quest = if requirements.contains_key("quest") {
                1_usize << 1
            } else {
                0
            };
            let setting = if requirements.contains_key("setting") {
                1_usize << 2
            } else {
                0
            };
            let shape = skill | quest | setting;
            if shape == 0 {
                return Err("bank result requires has no schema fields".into());
            }
            shape
        }
    };
    Ok(
        (approach * BANK_ACCESS_SHAPES + access_shape) * BANK_REQUIREMENT_SHAPES
            + requirement_shape,
    )
}

fn install_bank_result_shapes(runtime: &mut Runtime) -> Result<(), String> {
    let shapes = {
        let mut scope = runtime.deno_runtime().handle_scope();
        Rc::new(RefCell::new(BankResultShapes::create(&mut scope)?))
    };
    if runtime.deno_runtime().v8_isolate().set_slot(shapes) {
        Ok(())
    } else {
        Err("bank result shape cache already installed".into())
    }
}

const NEARBY_BANK_RADIUS: f64 = 14.0;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenRouteInput {
    #[serde(default)]
    bank_open: Option<bool>,
    #[serde(default)]
    here: Option<FromTile>,
    #[serde(default)]
    stand: Option<FromTile>,
    #[serde(default)]
    nearby_booth_dist: Option<f64>,
    #[serde(default)]
    nearest: Option<OpenRouteBank>,
    #[serde(default)]
    prefer_nearby: Option<bool>,
    #[serde(default)]
    nearby_radius: Option<f64>,
}

#[derive(Deserialize)]
struct OpenRouteBank {
    tile: FromTile,
}

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    install_bank_result_shapes(runtime)?;
    callback_v8::install(runtime, "__rs2b0t_bank_locations", catalog)?;
    callback_v8::install(runtime, "__rs2b0t_bank_unlocked", unlocked)?;
    callback_v8::install(runtime, "__rs2b0t_locked_bank_at", locked_at)?;
    callback_v8::install(runtime, "__rs2b0t_nearest_bank", nearest)?;
    callback_v8::install(runtime, "__rs2b0t_nearest_banks", ranked)?;
    callback_v8::install(runtime, "__rs2b0t_nearest_usable_bank", usable)?;
    callback_v8::install(runtime, "__rs2b0t_bank_materialize", bank_materialize)?;
    callback_v8::install(runtime, "__rs2b0t_bank_approach", bank_approach)?;
    callback_v8::install(runtime, "__rs2b0t_bank_distance", bank_distance)?;
    callback_v8::install(
        runtime,
        "__rs2b0t_resolve_bank_open_route",
        resolve_bank_open_route,
    )?;
    callback_v8::install(runtime, "__rs2b0t_cook_locations", cook_catalog)?;
    callback_v8::install(
        runtime,
        "__rs2b0t_cook_location_options",
        cook_location_options,
    )?;
    callback_v8::install(runtime, "__rs2b0t_find_cook_location", find_cook_location)?;
    callback_v8::install(
        runtime,
        "__rs2b0t_nearest_cook_surface",
        nearest_cook_surface,
    )?;
    callback_v8::install(runtime, "__rs2b0t_resolve_cook_location", resolve_cook)
}

fn origin<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> callback_v8::JsResult<'s, WorldTile> {
    let from = serde_v8::from_v8::<FromTile>(scope, value)
        .map_err(|e| callback_v8::type_error(scope, &format!("bank origin: {e}")))?;
    Ok(WorldTile {
        x: from.x,
        z: from.z,
        level: from.level,
    })
}

fn bank_materialize<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = materialize_bank(scope, args.get(1), args.get(0));
    callback_v8::finish(scope, rv, result);
}

fn bank_approach<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let bank = args.get(0);
        let approach = callback_v8::get(scope, bank, "approach")?;
        if approach.is_null_or_undefined() {
            callback_v8::get(scope, bank, "tile")
        } else {
            Ok(approach)
        }
    })();
    callback_v8::finish(scope, rv, result);
}

fn materialize_bank<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile_constructor: v8::Local<'s, v8::Value>,
    bank: v8::Local<'s, v8::Value>,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Value>> {
    if !bank.boolean_value(scope) {
        return Ok(v8::null(scope).into());
    }
    let row = v8::Local::<v8::Object>::try_from(bank)
        .map_err(|_| callback_v8::type_error(scope, "bank result is not an object"))?;
    let tile = callback_v8::get(scope, bank, "tile")?;
    let tile = tile_instance(scope, tile_constructor, tile)?;
    set_bank_field(scope, row, "tile", tile)?;
    let approach = callback_v8::get(scope, bank, "approach")?;
    if approach.boolean_value(scope) {
        let approach = tile_instance(scope, tile_constructor, approach)?;
        set_bank_field(scope, row, "approach", approach)?;
    }
    Ok(bank)
}

fn tile_instance<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile_constructor: v8::Local<'s, v8::Value>,
    tile: v8::Local<'s, v8::Value>,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Value>> {
    let x = callback_v8::get(scope, tile, "x")?;
    let z = callback_v8::get(scope, tile, "z")?;
    let level = callback_v8::get(scope, tile, "level")?;
    let level = if level.is_null_or_undefined() {
        v8::Integer::new(scope, 0).into()
    } else {
        level
    };
    callback_v8::construct(scope, tile_constructor, &[x, z, level], "Tile")
}

fn set_bank_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    row: v8::Local<'s, v8::Object>,
    key: &str,
    value: v8::Local<'s, v8::Value>,
) -> callback_v8::JsResult<'s, ()> {
    let name = callback_v8::string(scope, key);
    let set = callback_v8::catching(scope, |s| row.set(s, name, value))?;
    if !set {
        return Err(callback_v8::type_error(
            scope,
            "could not materialize bank tile",
        ));
    }
    Ok(())
}

fn bank_json_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: &serde_json::Value,
    field: &str,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Value>> {
    match value.get(field) {
        None => Ok(v8::undefined(scope).into()),
        Some(serde_json::Value::Null) => Ok(v8::null(scope).into()),
        Some(serde_json::Value::Bool(value)) => Ok(v8::Boolean::new(scope, *value).into()),
        Some(serde_json::Value::Number(value)) => {
            let Some(value) = value.as_f64() else {
                return Err(callback_v8::type_error(
                    scope,
                    "bank result: numeric field is not representable",
                ));
            };
            Ok(v8::Number::new(scope, value).into())
        }
        Some(serde_json::Value::String(value)) => {
            let Some(value) = v8::String::new(scope, value) else {
                return Err(callback_v8::type_error(
                    scope,
                    "bank result: string allocation failed",
                ));
            };
            Ok(value.into())
        }
        Some(serde_json::Value::Array(_) | serde_json::Value::Object(_)) => Err(
            callback_v8::type_error(scope, "bank result: expected a scalar field"),
        ),
    }
}

fn bank_tile_instance<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile_constructor: v8::Local<'s, v8::Value>,
    tile: &serde_json::Value,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Value>> {
    if !tile.is_object() {
        return Err(callback_v8::type_error(
            scope,
            "bank result: tile is not an object",
        ));
    }
    let x = bank_json_field(scope, tile, "x")?;
    let z = bank_json_field(scope, tile, "z")?;
    let level = match tile.get("level") {
        None | Some(serde_json::Value::Null) => v8::Integer::new(scope, 0).into(),
        Some(_) => bank_json_field(scope, tile, "level")?,
    };
    callback_v8::construct(scope, tile_constructor, &[x, z, level], "Tile")
}

fn set_bank_result_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    row: v8::Local<'s, v8::Object>,
    key: v8::Local<'s, v8::String>,
    value: v8::Local<'s, v8::Value>,
) -> callback_v8::JsResult<'s, ()> {
    if !callback_v8::catching(scope, |s| row.set(s, key.into(), value))? {
        return Err(callback_v8::type_error(
            scope,
            "bank result: could not set property",
        ));
    }
    Ok(())
}

fn new_bank_result_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    template: Option<v8::Local<'s, v8::ObjectTemplate>>,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Object>> {
    let Some(template) = template else {
        return Err(callback_v8::type_error(
            scope,
            "bank result: cached shape is incomplete",
        ));
    };
    template
        .new_instance(scope)
        .ok_or_else(|| callback_v8::type_error(scope, "bank result: object allocation failed"))
}

fn bank_value_v8<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile_constructor: v8::Local<'s, v8::Value>,
    value: serde_json::Value,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Value>> {
    if value.is_null() {
        return Ok(v8::null(scope).into());
    }
    let shape_index = bank_result_shape_index(&value)
        .map_err(|error| callback_v8::type_error(scope, &format!("bank result: {error}")))?;
    let Some(cache) = scope.get_slot::<Rc<RefCell<BankResultShapes>>>().cloned() else {
        return Err(callback_v8::type_error(
            scope,
            "bank result shape cache is not installed",
        ));
    };
    let (shape, keys) = {
        let mut cache = cache.borrow_mut();
        cache
            .shape(scope, &value, shape_index)
            .map_err(|error| callback_v8::type_error(scope, &format!("bank result: {error}")))?
    };
    let Some(fields) = value.as_object() else {
        return Err(callback_v8::type_error(
            scope,
            "bank result is not an object",
        ));
    };
    let row = new_bank_result_object(scope, Some(shape.row))?;
    let name = bank_json_field(scope, &value, "name")?;
    set_bank_result_field(scope, row, keys.name, name)?;
    let Some(tile_value) = fields.get("tile") else {
        return Err(callback_v8::type_error(
            scope,
            "bank result: tile is missing",
        ));
    };
    let tile = bank_tile_instance(scope, tile_constructor, tile_value)?;
    set_bank_result_field(scope, row, keys.tile, tile)?;
    if let Some(approach_value) = fields.get("approach") {
        let approach = bank_tile_instance(scope, tile_constructor, approach_value)?;
        set_bank_result_field(scope, row, keys.approach, approach)?;
    }
    if let Some(access_value) = fields.get("npcAccess") {
        let Some(access_fields) = access_value.as_object() else {
            return Err(callback_v8::type_error(
                scope,
                "bank result npcAccess is not an object",
            ));
        };
        let access = new_bank_result_object(scope, shape.access)?;
        let name = bank_json_field(scope, access_value, "name")?;
        set_bank_result_field(scope, access, keys.name, name)?;
        let op = bank_json_field(scope, access_value, "op")?;
        set_bank_result_field(scope, access, keys.op, op)?;
        if access_fields.contains_key("choose") {
            let choose = bank_json_field(scope, access_value, "choose")?;
            set_bank_result_field(scope, access, keys.choose, choose)?;
        }
        set_bank_result_field(scope, row, keys.npc_access, access.into())?;
    } else if let Some(access_value) = fields.get("access") {
        let Some(access_fields) = access_value.as_object() else {
            return Err(callback_v8::type_error(
                scope,
                "bank result access is not an object",
            ));
        };
        let access = new_bank_result_object(scope, shape.access)?;
        let name = bank_json_field(scope, access_value, "name")?;
        set_bank_result_field(scope, access, keys.name, name)?;
        let op = bank_json_field(scope, access_value, "op")?;
        set_bank_result_field(scope, access, keys.op, op)?;
        if let Some(open_first_value) = access_fields.get("openFirst") {
            if !open_first_value.is_object() {
                return Err(callback_v8::type_error(
                    scope,
                    "bank result openFirst is not an object",
                ));
            }
            let open_first = new_bank_result_object(scope, shape.open_first)?;
            let name = bank_json_field(scope, open_first_value, "name")?;
            set_bank_result_field(scope, open_first, keys.name, name)?;
            let op = bank_json_field(scope, open_first_value, "op")?;
            set_bank_result_field(scope, open_first, keys.op, op)?;
            set_bank_result_field(scope, access, keys.open_first, open_first.into())?;
        }
        set_bank_result_field(scope, row, keys.access, access.into())?;
    }
    if let Some(requirements_value) = fields.get("requires") {
        let Some(requirements_fields) = requirements_value.as_object() else {
            return Err(callback_v8::type_error(
                scope,
                "bank result requires is not an object",
            ));
        };
        let requires = new_bank_result_object(scope, shape.requires)?;
        if let Some(skill_value) = requirements_fields.get("skill") {
            let skill = new_bank_result_object(scope, shape.skill)?;
            let name = bank_json_field(scope, skill_value, "name")?;
            set_bank_result_field(scope, skill, keys.name, name)?;
            let level = bank_json_field(scope, skill_value, "level")?;
            set_bank_result_field(scope, skill, keys.level, level)?;
            set_bank_result_field(scope, requires, keys.skill, skill.into())?;
        }
        if requirements_fields.contains_key("quest") {
            let quest = bank_json_field(scope, requirements_value, "quest")?;
            set_bank_result_field(scope, requires, keys.quest, quest)?;
        }
        if requirements_fields.contains_key("setting") {
            let setting = bank_json_field(scope, requirements_value, "setting")?;
            set_bank_result_field(scope, requires, keys.setting, setting)?;
        }
        set_bank_result_field(scope, row, keys.requires, requires.into())?;
    }
    Ok(row.into())
}

fn bank_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile_constructor: v8::Local<'s, v8::Value>,
    banks: Vec<api::named_banks::NamedBank>,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Value>> {
    let length = i32::try_from(banks.len())
        .map_err(|_| callback_v8::type_error(scope, "bank results exceed V8 array length"))?;
    let values = v8::Array::new(scope, length);
    for (index, bank) in banks.into_iter().enumerate() {
        let index = u32::try_from(index)
            .map_err(|_| callback_v8::type_error(scope, "bank results exceed V8 array limit"))?;
        let value = bank_value_v8(scope, tile_constructor, bank_select::bank_value(&bank))?;
        if !callback_v8::catching(scope, |s| values.set_index(s, index, value))? {
            return Err(callback_v8::type_error(
                scope,
                "could not materialize bank results",
            ));
        }
    }
    Ok(values.into())
}

fn catalog<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = bank_array(scope, args.get(0), bank_select::banks());
    callback_v8::finish(scope, rv, result);
}

fn unlocked<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    #[derive(Deserialize)]
    struct Identity {
        name: String,
        x: i32,
        z: i32,
        #[serde(default)]
        level: i32,
    }
    let result = (|| {
        let bank = serde_v8::from_v8::<Identity>(scope, args.get(0))
            .map_err(|e| callback_v8::type_error(scope, &format!("bank identity: {e}")))?;
        Ok(v8::Boolean::new(
            scope,
            bank_select::unlocked(
                &bank.name,
                WorldTile {
                    x: bank.x,
                    z: bank.z,
                    level: bank.level,
                },
            ),
        )
        .into())
    })();
    callback_v8::finish(scope, rv, result);
}

fn locked_at<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let tile = origin(scope, args.get(0))?;
        bank_value_v8(scope, args.get(1), bank_select::locked_bank_at(tile))
    })();
    callback_v8::finish(scope, rv, result);
}

/// Frozen `bankDistance` and pure `resolveBankOpenRoute` share Euclidean x/z
/// distance, independent of plane.
fn bank_distance<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let from = origin(scope, args.get(0))?;
        let bank = origin(scope, args.get(1))?;
        Ok(v8::Number::new(scope, bank_distance_value(&from, &bank)).into())
    })();
    callback_v8::finish(scope, rv, result);
}

fn bank_distance_value(from: &WorldTile, bank: &WorldTile) -> f64 {
    let dx = f64::from(bank.x) - f64::from(from.x);
    let dz = f64::from(bank.z) - f64::from(from.z);
    script::gather_locations::distance_delta(dx, dz)
}

fn from_tile(tile: &FromTile) -> WorldTile {
    WorldTile {
        x: tile.x,
        z: tile.z,
        level: tile.level,
    }
}

fn resolve_bank_open_route<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let input = args.get(0);
        if input.is_string() && input.to_rust_string_lossy(scope) == "nearbyRadius" {
            return Ok(v8::Number::new(scope, NEARBY_BANK_RADIUS).into());
        }
        let input = serde_v8::from_v8::<OpenRouteInput>(scope, input)
            .map_err(|e| callback_v8::type_error(scope, &format!("bank route input: {e}")))?;
        serde_v8::to_v8(scope, select_open_route(&input))
            .map_err(|e| callback_v8::type_error(scope, &format!("bank route result: {e}")))
    })();
    callback_v8::finish(scope, rv, result);
}

fn select_open_route(input: &OpenRouteInput) -> &'static str {
    if input.bank_open.unwrap_or(false) {
        return "already-open";
    }
    let prefer_nearby = input.prefer_nearby != Some(false);
    let radius = input.nearby_radius.unwrap_or(NEARBY_BANK_RADIUS);
    if prefer_nearby
        && input
            .nearby_booth_dist
            .is_some_and(|distance| distance <= radius)
    {
        return "scene-booth";
    }
    if prefer_nearby {
        if let (Some(here), Some(nearest)) = (input.here.as_ref(), input.nearest.as_ref()) {
            let here = from_tile(here);
            if bank_distance_value(&here, &from_tile(&nearest.tile)) <= radius {
                let preset_is_far = match input.stand.as_ref() {
                    Some(stand) => bank_distance_value(&here, &from_tile(stand)) > radius,
                    None => true,
                };
                if preset_is_far {
                    return "local-bank";
                }
            }
        }
    }
    if input.stand.is_some() {
        "preset-stand"
    } else {
        "nearest-fallback"
    }
}

fn nearest<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let from = origin(scope, args.get(0))?;
        bank_value_v8(scope, args.get(1), bank_select::nearest(from))
    })();
    callback_v8::finish(scope, rv, result);
}

fn ranked<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let from = origin(scope, args.get(0))?;
        bank_array(scope, args.get(1), bank_select::ranked(from))
    })();
    callback_v8::finish(scope, rv, result);
}

fn usable<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let from = origin(scope, args.get(0))?;
        let predicate = callback_v8::Callback::plain(scope, args.get(1), "usable");
        let tile_constructor = args.get(2);
        let mut best = None;
        let mut distance = i64::MAX;
        for bank in bank_select::banks() {
            let candidate = bank_value_v8(scope, tile_constructor, bank_select::bank_value(&bank))?;
            let accepted = predicate.call(scope, &[candidate])?;
            if accepted.boolean_value(scope) {
                let candidate_distance =
                    script::native_bank::air_distance_squared(from, bank.air_tile());
                if candidate_distance < distance {
                    distance = candidate_distance;
                    best = Some(bank);
                }
            }
        }
        match best {
            Some(bank) => bank_value_v8(scope, tile_constructor, bank_select::bank_value(&bank)),
            None => Ok(v8::null(scope).into()),
        }
    })();
    callback_v8::finish(scope, rv, result);
}

/// The bank roster paired with its cook surfaces; `None` without selected
/// game data.
fn cook_roster() -> Option<(Vec<api::named_banks::NamedBank>, Vec<CookLocation>)> {
    let data = crate::supply_v2::selected_data()?;
    let banks = bank_select::banks();
    let locations = cook_locations::build(&banks, data.cook_surfaces());
    Some((banks, locations))
}

/// Frozen `COOK_LOCATIONS` (`api/cooking/CookLocations.ts:12`): one row per
/// roster bank, `bank` its index in `BANK_LOCATIONS`. Every shim module
/// evaluates at load, so without game data the table is empty (like
/// `ITEM_DB`) and `resolveCookLocation` reports the missing data.
fn cook_catalog<'s>(
    scope: &mut v8::HandleScope<'s>,
    _args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let locations = cook_roster().map_or_else(Vec::new, |(_, locations)| locations);
    let tile = |tile: WorldTile| json!({"x": tile.x, "z": tile.z, "level": tile.level});
    let rows: Vec<_> = locations
        .iter()
        .map(|location| {
            let surface = location.surface.as_ref().map(|plan| {
                let mut surface = json!({
                    "stand": tile(plan.stand),
                    "locName": plan.loc_name,
                    "kind": plan.kind.as_str(),
                    "loc": tile(plan.loc),
                    "arriveRadius": plan.arrive_radius,
                    "label": plan.label,
                });
                if let Some(approach) = plan.approach {
                    surface["approach"] = tile(approach);
                }
                surface
            });
            json!({
                "bank": location.bank,
                "name": location.name,
                "surface": surface,
                "obstacles": api::cook_locations::DEFAULT_OBSTACLES,
                "verified": location.verified,
            })
        })
        .collect();
    let result = serde_v8::to_v8(scope, rows)
        .map_err(|e| callback_v8::type_error(scope, &format!("cook locations: {e}")));
    callback_v8::finish(scope, rv, result);
}

/// Native projection of `['Auto', ...locations.map(l => l.name), custom]`.
fn cook_location_options<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let locations = v8::Local::<v8::Array>::try_from(args.get(0))
            .map_err(|_| callback_v8::type_error(scope, "cook locations are not an array"))?;
        let options = v8::Array::new(scope, 0);
        let auto = callback_v8::string(scope, "Auto");
        set_array_data(scope, options, 0, auto)?;
        for index in 0..locations.length() {
            let location = callback_v8::get_index(scope, locations.into(), index)?;
            let name = callback_v8::get(scope, location, "name")?;
            set_array_data(scope, options, index + 1, name)?;
        }
        let custom = args.get(1);
        set_array_data(scope, options, locations.length() + 1, custom)?;
        Ok(options.into())
    })();
    callback_v8::finish(scope, rv, result);
}

fn set_array_data<'s>(
    scope: &mut v8::HandleScope<'s>,
    array: v8::Local<'s, v8::Array>,
    index: u32,
    value: v8::Local<'s, v8::Value>,
) -> callback_v8::JsResult<'s, ()> {
    let key = v8::Integer::new_from_unsigned(scope, index)
        .to_string(scope)
        .ok_or_else(|| callback_v8::type_error(scope, "cook location option index"))?;
    let created = callback_v8::catching(scope, |scope| {
        array.create_data_property(scope, key.cast(), value)
    })?;
    if created {
        Ok(())
    } else {
        Err(callback_v8::type_error(
            scope,
            "could not create cook location option",
        ))
    }
}

fn string_constructor<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    callback_v8::get(scope, global.into(), "String")
}

fn call_string<'s>(
    scope: &mut v8::HandleScope<'s>,
    constructor: v8::Local<'s, v8::Value>,
    value: v8::Local<'s, v8::Value>,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Value>> {
    let undefined = v8::undefined(scope).into();
    callback_v8::call(scope, constructor, undefined, &[value], "String")
}

/// Preserve the frozen `String` conversions and caller-owned `find` method;
/// the row-matching predicate itself runs in this native callback.
fn find_cook_location<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let constructor = string_constructor(scope)?;
        let wanted = call_string(scope, constructor, args.get(1))?;
        let wanted = callback_v8::call_method(scope, wanted, "trim", &[], "String(name).trim")?;
        let wanted = callback_v8::call_method(
            scope,
            wanted,
            "toLowerCase",
            &[],
            "String(name).trim().toLowerCase",
        )?;
        let locations = args.get(0);
        let locations = if callback_v8::truthy(scope, locations) {
            locations
        } else {
            v8::Array::new(scope, 0).into()
        };
        let predicate = v8::Function::builder(cook_location_matches)
            .data(wanted)
            .build(scope)
            .ok_or_else(|| {
                callback_v8::type_error(scope, "could not create cook location matcher")
            })?;
        let found =
            callback_v8::call_method(scope, locations, "find", &[predicate.into()], "locs.find")?;
        Ok(if callback_v8::truthy(scope, found) {
            found
        } else {
            v8::null(scope).into()
        })
    })();
    callback_v8::finish(scope, rv, result);
}

fn cook_location_matches<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let location = args.get(0);
        if !callback_v8::truthy(scope, location) {
            return Ok(location);
        }
        let constructor = string_constructor(scope)?;
        let name = callback_v8::get(scope, location, "name")?;
        let name = call_string(scope, constructor, name)?;
        let name = callback_v8::call_method(
            scope,
            name,
            "toLowerCase",
            &[],
            "String(l.name).toLowerCase",
        )?;
        Ok(v8::Boolean::new(scope, name.strict_equals(args.data())).into())
    })();
    callback_v8::finish(scope, rv, result);
}

fn nearest_cook_surface<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let Some(data) = crate::supply_v2::selected_data() else {
            return Ok(v8::null(scope).into());
        };
        let surfaces = data.cook_surfaces();
        if surfaces.is_empty() {
            return Ok(v8::null(scope).into());
        }
        let origin = origin(scope, args.get(0))?;
        let max_cheb = args.get(1);
        let max_cheb = if max_cheb.is_undefined() {
            f64::from(api::cook_locations::MAX_SURFACE_CHEB)
        } else {
            callback_v8::number(scope, max_cheb)?
        };
        let Some(surface) = cook_locations::nearest_surface(origin, surfaces, max_cheb) else {
            return Ok(v8::null(scope).into());
        };
        let value = json!({
            "x": surface.x,
            "z": surface.z,
            "level": surface.level,
            "name": surface.name,
            "debugname": surface.debugname,
            "kind": surface.kind.as_str(),
        });
        serde_v8::to_v8(scope, value)
            .map_err(|e| callback_v8::type_error(scope, &format!("cook surface: {e}")))
    })();
    callback_v8::finish(scope, rv, result);
}

/// Frozen `resolveCookLocation(setting, from, unlocked?)`
/// (`api/cooking/CookLocations.ts:25-49`, [`cook_locations::resolve`]).
/// `unlocked` defaults to the bank's own requirement (`:28`); a caller's
/// predicate is called with the location's index, which the shim maps to
/// its `CookLocation`. Answers that index or `null`.
fn resolve_cook<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let setting = callback_v8::to_string(scope, args.get(0))?;
        let Some(wanted) = cook_locations::wanted(&setting) else {
            return Ok(None);
        };
        let Some((banks, locations)) = cook_roster() else {
            let message = v8::String::new(scope, crate::supply_v2::GAME_DATA_UNAVAILABLE)
                .unwrap_or_else(|| v8::String::empty(scope));
            return Err(callback_v8::Throw::Value(v8::Exception::error(
                scope, message,
            )));
        };
        let predicate = args.get(2);
        let predicate = (!predicate.is_undefined())
            .then(|| callback_v8::Callback::plain(scope, predicate, "unlocked"));
        cook_locations::resolve(
            &locations,
            &banks,
            wanted,
            scope,
            |scope| origin(scope, args.get(1)),
            |scope, index| match &predicate {
                Some(predicate) => {
                    let index = callback_v8::num(scope, index as f64);
                    predicate.truthy(scope, &[index])
                }
                None => Ok(bank_select::bank_unlocked(&banks[locations[index].bank])),
            },
        )
    })()
    .map(|index| match index {
        Some(index) => callback_v8::num(scope, index as f64),
        None => v8::null(scope).into(),
    });
    callback_v8::finish(scope, rv, result);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyscript::RuntimeOptions;

    const COOKING_SETUP: &str = r#"
const cookingRows = [
    { name: 'Alpha' },
    { name: 'alpha' },
    { name: 'Beta' },
];
globalThis.checkCookingHelpers = () => {
    const options = __rs2b0t_cook_location_options(cookingRows, 'Custom');
    return options.length === 5 &&
        options[0] === 'Auto' && options[1] === 'Alpha' &&
        options[2] === 'alpha' && options[3] === 'Beta' &&
        options[4] === 'Custom' &&
        __rs2b0t_find_cook_location(cookingRows, ' ALPHA ') === cookingRows[0] &&
        __rs2b0t_find_cook_location(cookingRows, 'beta') === cookingRows[2] &&
        __rs2b0t_find_cook_location(cookingRows, 'missing') === null &&
        __rs2b0t_find_cook_location(null, 'Alpha') === null;
};
"#;

    fn setup() -> Runtime {
        crate::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        callback_v8::install_ops(&mut runtime).unwrap();
        install(&mut runtime).unwrap();
        runtime.eval::<()>(COOKING_SETUP).unwrap();
        runtime
    }

    #[test]
    fn cooking_local_helpers_bound_cold_cache_and_keep_zero_warm_rust_heap() {
        let mut runtime = setup();
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let key = callback_v8::string(&mut scope, "checkCookingHelpers");
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
        println!("Cooking option/lookup callbacks cold={cold:?}; warm1000={warm:?}");
        assert!(cold.count_total <= 1);
        assert!(cold.count_current <= 1);
        assert!(cold.count_max <= 1);
        assert!(cold.bytes_total <= 112);
        assert!(cold.bytes_current <= 112);
        assert!(cold.bytes_max <= 112);
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
    fn cooking_cold_charge_reuses_an_independent_scope_cache_cell() {
        let mut runtime = setup();
        callback_v8::install(&mut runtime, "__scope_cache_control", scope_cache_control).unwrap();
        runtime
            .eval::<()>(
                r#"
globalThis.checkPureJs = () => true;
globalThis.checkThreeScopes = () => __scope_cache_control(false);
globalThis.checkFourScopes = () => __scope_cache_control(true);
"#,
            )
            .unwrap();
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let key = callback_v8::string(&mut scope, "checkPureJs");
        let pure_js =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let key = callback_v8::string(&mut scope, "checkThreeScopes");
        let three_scopes =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let key = callback_v8::string(&mut scope, "checkFourScopes");
        let four_scopes =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let key = callback_v8::string(&mut scope, "checkCookingHelpers");
        let cooking =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let receiver = v8::undefined(&mut scope).into();
        let pure = allocation_counter::measure(|| {
            assert!(pure_js.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let cold = allocation_counter::measure(|| {
            assert!(three_scopes
                .call(&mut scope, receiver, &[])
                .unwrap()
                .is_true());
        });
        let cooking_after_control = allocation_counter::measure(|| {
            assert!(cooking.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let extra_depth = allocation_counter::measure(|| {
            assert!(four_scopes
                .call(&mut scope, receiver, &[])
                .unwrap()
                .is_true());
        });
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(four_scopes
                    .call(&mut scope, receiver, &[])
                    .unwrap()
                    .is_true());
                assert!(cooking.call(&mut scope, receiver, &[]).unwrap().is_true());
            }
        });
        println!(
            "Cooking scope-cache control pure={pure:?}; three scopes={cold:?}; \
             cooking after control={cooking_after_control:?}; \
             extra scope={extra_depth:?}; warm1000={warm:?}"
        );
        assert_eq!(pure, allocation_counter::AllocationInfo::default());
        assert_eq!(cold.count_total, 1);
        assert_eq!(cold.count_current, 1);
        assert_eq!(cold.bytes_total, 112);
        assert_eq!(cold.bytes_total, extra_depth.bytes_total);
        assert_eq!(
            cooking_after_control,
            allocation_counter::AllocationInfo::default()
        );
        assert_eq!(extra_depth.count_total, 1);
        assert_eq!(extra_depth.count_current, 1);
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }
    const BANK_SHAPE_TEST_NAME: &str =
        "bank_locations_v8::tests::bank_result_shapes_cover_callbacks_and_bound_v8_heap";
    const BANK_SHAPE_CHILD_ENV: &str = "SCRIPT_BANK_RESULT_SHAPE_TEST_CHILD";
    const BANK_RESULT_SHAPE_SETUP: &str = r#"
globalThis.__bank_tile_calls = 0;
globalThis.__bank_predicate_calls = 0;
globalThis.__BankTile = class BankTile {
    constructor(x, z, level) {
        this.x = x;
        this.z = z;
        this.level = level;
        globalThis.__bank_tile_calls++;
    }
};
globalThis.__checkBankResultShapes = () => {
    const hasFastProperties = new Function('value', 'return %HasFastProperties(value)');
    const expected = globalThis.__bank_expected;
    const expectedByName = new Map(expected.map(row => [row.name, row]));
    const fail = message => { throw new Error(message); };
    const require = (condition, message) => { if (!condition) fail(message); };
    const has = (value, key) => Object.prototype.hasOwnProperty.call(value, key);
    const checkRecord = (actual, wanted, label) => {
        require(actual && typeof actual === 'object', `${label} is not an object`);
        require(hasFastProperties(actual), `${label} does not have fast properties`);
        require(Object.getPrototypeOf(actual) === Object.prototype, `${label} prototype changed`);
        const keys = Reflect.ownKeys(actual);
        require(JSON.stringify(keys) === JSON.stringify(Reflect.ownKeys(wanted)),
            `${label} own-property order changed`);
        for (const key of keys) {
            const descriptor = Object.getOwnPropertyDescriptor(actual, key);
            require(Object.prototype.hasOwnProperty.call(descriptor, 'value') &&
                descriptor.writable && descriptor.enumerable && descriptor.configurable,
                `${label}.${key} descriptor changed`);
        }
    };
    const checkTile = (actual, wanted, label) => {
        require(actual instanceof __BankTile, `${label} is not a Tile`);
        require(hasFastProperties(actual), `${label} does not have fast properties`);
        const keys = Reflect.ownKeys(actual);
        require(JSON.stringify(keys) === JSON.stringify(Reflect.ownKeys(wanted)),
            `${label} own-property order changed`);
        for (const key of keys) {
            const descriptor = Object.getOwnPropertyDescriptor(actual, key);
            require(Object.prototype.hasOwnProperty.call(descriptor, 'value') &&
                descriptor.writable && descriptor.enumerable && descriptor.configurable,
                `${label}.${key} descriptor changed`);
        }
    };
    const checkBank = row => {
        const wanted = expectedByName.get(row.name);
        require(wanted, `unexpected bank ${row.name}`);
        checkRecord(row, wanted, `bank ${row.name}`);
        checkTile(row.tile, wanted.tile, `${row.name}.tile`);
        require(JSON.stringify(row) === JSON.stringify(wanted),
            `${row.name} values or nested property order changed`);
        if (has(wanted, 'approach')) {
            checkTile(row.approach, wanted.approach, `${row.name}.approach`);
        }
        for (const key of ['access', 'npcAccess', 'requires']) {
            if (!has(wanted, key)) continue;
            checkRecord(row[key], wanted[key], `${row.name}.${key}`);
            if (key === 'access' && has(wanted.access, 'openFirst')) {
                checkRecord(row.access.openFirst, wanted.access.openFirst,
                    `${row.name}.access.openFirst`);
            }
            if (key === 'requires' && has(wanted.requires, 'skill')) {
                checkRecord(row.requires.skill, wanted.requires.skill,
                    `${row.name}.requires.skill`);
            }
        }
    };
    const tileCallsFor = rows => rows.reduce((count, row) =>
        count + 1 + (has(row, 'approach') ? 1 : 0), 0);
    const checkCalls = (before, expectedCalls, label) =>
        require(globalThis.__bank_tile_calls - before === expectedCalls,
            `${label} called the Tile constructor an unexpected number of times`);

    require(expected.some(row => has(row, 'approach')), 'approach variant missing');
    require(expected.some(row => row.npcAccess && has(row.npcAccess, 'choose')),
        'npcAccess.choose variant missing');
    require(expected.some(row => row.access && has(row.access, 'openFirst')),
        'access.openFirst variant missing');
    require(expected.some(row => row.requires && has(row.requires, 'skill')),
        'requires.skill variant missing');
    require(expected.some(row => row.requires && has(row.requires, 'quest')),
        'requires.quest variant missing');
    require(expected.some(row => row.requires && has(row.requires, 'setting')),
        'requires.setting variant missing');

    let before = globalThis.__bank_tile_calls;
    const catalog = __rs2b0t_bank_locations(__BankTile);
    require(Array.isArray(catalog) && catalog.length === expected.length,
        'catalog callback result changed');
    catalog.forEach(checkBank);
    require(JSON.stringify(catalog) === JSON.stringify(expected),
        'catalog values or row order changed');
    checkCalls(before, tileCallsFor(catalog), 'catalog');

    before = globalThis.__bank_tile_calls;
    const ranked = __rs2b0t_nearest_banks({x: 3200, z: 3200, level: 0}, __BankTile);
    require(Array.isArray(ranked), 'ranked callback did not return an array');
    ranked.forEach(checkBank);
    checkCalls(before, tileCallsFor(ranked), 'ranked');

    before = globalThis.__bank_tile_calls;
    const nearest = __rs2b0t_nearest_bank({x: 3200, z: 3200, level: 0}, __BankTile);
    require(nearest !== null, 'nearest callback did not return an eligible bank');
    checkBank(nearest);
    checkCalls(before, tileCallsFor([nearest]), 'nearest');

    before = globalThis.__bank_tile_calls;
    const locked = __rs2b0t_locked_bank_at({x: 3153, z: 9576, level: 0}, __BankTile);
    require(locked !== null, 'locked-at callback did not return its gated bank');
    checkBank(locked);
    checkCalls(before, tileCallsFor([locked]), 'locked-at');

    const predicateBefore = globalThis.__bank_predicate_calls;
    before = globalThis.__bank_tile_calls;
    const usable = __rs2b0t_nearest_usable_bank(
        {x: 3200, z: 3200, level: 0},
        bank => { globalThis.__bank_predicate_calls++; return true; },
        __BankTile
    );
    require(usable !== null, 'usable callback did not return a bank');
    checkBank(usable);
    require(globalThis.__bank_predicate_calls - predicateBefore === expected.length,
        'usable predicate call count changed');
    checkCalls(before, tileCallsFor(expected) + tileCallsFor([usable]), 'usable');
    return true;
};
globalThis.__bank_warm = () => __rs2b0t_bank_locations(__BankTile).length;
globalThis.__bank_repeat = count => {
    for (let index = 0; index < count; index++) {
        globalThis.__bank_last = __rs2b0t_bank_locations(__BankTile);
    }
};
globalThis.__checkEmptyBankResults = () => {
    const before = globalThis.__bank_tile_calls;
    const origin = {x: 3200, z: 3200, level: 0};
    return __rs2b0t_bank_locations(__BankTile).length === 0 &&
        __rs2b0t_nearest_banks(origin, __BankTile).length === 0 &&
        __rs2b0t_nearest_bank(origin, __BankTile) === null &&
        __rs2b0t_locked_bank_at(origin, __BankTile) === null &&
        __rs2b0t_nearest_usable_bank(origin, () => true, __BankTile) === null &&
        globalThis.__bank_tile_calls === before;
};
"#;

    fn test_bank_facts() -> api::named_banks::NamedBankFacts {
        let mut banks = api::named_banks::BANK_CATALOG
            .iter()
            .map(|definition| api::named_banks::NamedBank {
                name: definition.name,
                tile: definition.tile,
                definition: Some(definition),
                routable: true,
            })
            .collect::<Vec<_>>();
        banks.push(api::named_banks::NamedBank::new(
            "Undeclared bank",
            WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
        ));
        api::named_banks::NamedBankFacts::from_banks(banks)
    }

    struct ResetBankSelection;

    impl Drop for ResetBankSelection {
        fn drop(&mut self) {
            bank_select::install(std::sync::Arc::new(
                api::named_banks::NamedBankFacts::empty(),
            ));
            bank_select::settings(&serde_json::Map::<String, serde_json::Value>::new());
        }
    }

    fn cached_bank_shape_count(runtime: &mut Runtime) -> usize {
        let scope = runtime.deno_runtime().handle_scope();
        let cache = scope
            .get_slot::<Rc<RefCell<BankResultShapes>>>()
            .cloned()
            .expect("bank result shape cache installed");
        let count = cache
            .borrow()
            .shapes
            .iter()
            .filter(|shape| shape.is_some())
            .count();
        count
    }

    fn expected_bank_shape_count(values: &[serde_json::Value]) -> usize {
        let mut shapes = [false; BANK_RESULT_SHAPE_COUNT];
        for value in values {
            let index = bank_result_shape_index(value).expect("bank schema shape");
            shapes[index] = true;
        }
        shapes.iter().filter(|present| **present).count()
    }

    fn bank_result_object_count() -> usize {
        let mut count = 1 + 2 * (api::named_banks::BANK_CATALOG.len() + 1);
        for definition in api::named_banks::BANK_CATALOG {
            if definition.approach.is_some() {
                count += 1;
            }
            if definition.npc.is_some() || definition.object.is_some() {
                count += 1;
            }
            if definition.open_first.is_some() {
                count += 1;
            }
            if definition.skill.is_some()
                || definition.quest.is_some()
                || definition.setting.is_some()
            {
                count += 1;
            }
            if definition.skill.is_some() {
                count += 1;
            }
        }
        count
    }

    fn collected_used_heap(runtime: &mut Runtime) -> usize {
        let isolate = runtime.deno_runtime().v8_isolate();
        isolate.low_memory_notification();
        isolate.get_heap_statistics().used_heap_size()
    }

    #[test]
    fn bank_result_shapes_cover_callbacks_and_bound_v8_heap() {
        if std::env::var_os(BANK_SHAPE_CHILD_ENV).is_none() {
            let output = std::process::Command::new(
                std::env::current_exe().expect("unit-test executable path"),
            )
            .args(["--exact", BANK_SHAPE_TEST_NAME, "--nocapture"])
            .env(BANK_SHAPE_CHILD_ENV, "1")
            .output()
            .expect("run isolated V8-native-syntax test");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success(),
                "isolated V8 shape test failed\nstdout: {stdout}\nstderr: {stderr}"
            );
            assert!(
                stdout.contains(&format!("test {BANK_SHAPE_TEST_NAME} ... ok"))
                    && stdout.contains("1 passed"),
                "isolated test did not run exactly once\nstdout: {stdout}\nstderr: {stderr}"
            );
            return;
        }

        v8::V8::set_flags_from_string("--allow-natives-syntax");
        let _reset = ResetBankSelection;
        bank_select::settings(&serde_json::Map::<String, serde_json::Value>::new());
        bank_select::install(std::sync::Arc::new(test_bank_facts()));
        let expected = bank_select::banks()
            .iter()
            .map(bank_select::bank_value)
            .collect::<Vec<_>>();
        let expected_shape_count = expected_bank_shape_count(&expected);
        let expected_json = serde_json::to_string(&expected).expect("serialize expected banks");
        let source =
            format!("globalThis.__bank_expected = {expected_json};\n{BANK_RESULT_SHAPE_SETUP}");
        let mut runtime = setup();
        runtime.eval::<()>(&source).unwrap();
        assert_eq!(cached_bank_shape_count(&mut runtime), 0);
        assert!(runtime.eval::<bool>("__checkBankResultShapes()").unwrap());
        let warmed_shapes = cached_bank_shape_count(&mut runtime);
        assert_eq!(warmed_shapes, expected_shape_count);
        assert!(warmed_shapes <= BANK_RESULT_SHAPE_COUNT);

        runtime
            .eval::<()>("globalThis.__bank_last = null; __bank_warm();")
            .unwrap();
        runtime
            .eval::<()>("globalThis.__bank_last = null;")
            .unwrap();
        let baseline = collected_used_heap(&mut runtime);
        runtime.eval::<()>("__bank_repeat(1000);").unwrap();
        let after_repeat = collected_used_heap(&mut runtime);
        let growth = after_repeat.saturating_sub(baseline);
        // `cold_native_entity_rows_bound_collected_v8_heap_growth` caps
        // collected growth at 4 MiB for its 4,096-row maximum (after a 64-row
        // baseline). Charge each retained bank-result object at 1 KiB.
        const COMPARABLE_ENTITY_HEAP_BOUND: usize = 4_194_304;
        const COMPARABLE_ENTITY_ROWS: usize = 4_096;
        let growth_limit =
            bank_result_object_count() * (COMPARABLE_ENTITY_HEAP_BOUND / COMPARABLE_ENTITY_ROWS);
        println!(
            "Bank result collected V8 heap after warmup: rows={} objects={} baseline={} \
             after_repeat={} growth={} limit={growth_limit}",
            api::named_banks::BANK_CATALOG.len() + 1,
            bank_result_object_count(),
            baseline,
            after_repeat,
            growth
        );
        assert!(
            growth <= growth_limit,
            "repeated bank materialization grew collected V8 used heap by {growth} bytes; \
             limit {growth_limit} (baseline {baseline}, after {after_repeat})"
        );
        assert_eq!(cached_bank_shape_count(&mut runtime), warmed_shapes);

        bank_select::install(std::sync::Arc::new(
            api::named_banks::NamedBankFacts::empty(),
        ));
        assert!(runtime.eval::<bool>("__checkEmptyBankResults()").unwrap());
        assert_eq!(cached_bank_shape_count(&mut runtime), warmed_shapes);
    }
}
