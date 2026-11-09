//! The selected walk destination table, materialized once per isolate from the native shared view.
//!
//! The script-visible records use one cached object template. The shim does not map or clone the table in JavaScript.

use super::callback_v8::{self as cb, JsResult};
use rustyscript::Runtime;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use std::{cell::RefCell, rc::Rc};

type Value<'s> = v8::Local<'s, v8::Value>;
type Shared = Rc<RefCell<Catalog>>;

#[cfg(test)]
static CALLBACK_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
struct Catalog {
    row_shape: Option<v8::Global<v8::ObjectTemplate>>,
    values: Option<v8::Global<v8::Array>>,
}

pub(crate) fn install(runtime: &mut Runtime) -> Result<(), String> {
    if !runtime
        .deno_runtime()
        .v8_isolate()
        .set_slot(Shared::default())
    {
        return Err("walk destination cache already installed".into());
    }
    cb::install(runtime, "__rs2b0t_walk_catalog", catalog_callback)
}

fn catalog_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    #[cfg(test)]
    CALLBACK_COUNT.fetch_add(1, Ordering::Relaxed);
    let result = materialize(scope, &args);
    cb::finish(scope, rv, result);
}

fn shared<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, Shared> {
    scope
        .get_slot::<Shared>()
        .cloned()
        .ok_or_else(|| cb::not_impl(scope, "walk destinations"))
}

fn string<'s>(scope: &mut v8::HandleScope<'s>, value: &str) -> v8::Local<'s, v8::String> {
    v8::String::new_from_utf8(scope, value.as_bytes(), v8::NewStringType::Internalized)
        .unwrap_or_else(|| v8::String::empty(scope))
}

fn integer<'s>(scope: &mut v8::HandleScope<'s>, value: i32) -> Value<'s> {
    v8::Integer::new(scope, value).into()
}

fn template<'s>(scope: &mut v8::HandleScope<'s>) -> v8::Local<'s, v8::ObjectTemplate> {
    let template = v8::ObjectTemplate::new(scope);
    let placeholder: v8::Local<v8::Data> = v8::undefined(scope).into();
    for field in ["name", "tile"] {
        template.set(string(scope, field).into(), placeholder);
    }
    template
}

fn row<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: v8::Local<'s, v8::String>,
    tile: Value<'s>,
) -> JsResult<'s, Value<'s>> {
    let shared = shared(scope)?;
    let cached = shared
        .borrow()
        .row_shape
        .as_ref()
        .map(|shape| v8::Local::new(scope, shape));
    let shape = match cached {
        Some(shape) => shape,
        None => {
            let shape = template(scope);
            shared.borrow_mut().row_shape = Some(v8::Global::new(scope, shape));
            shape
        }
    };
    let object = shape
        .new_instance(scope)
        .ok_or_else(|| cb::type_error(scope, "walk destinations: object allocation failed"))?;
    for (field, value) in [("name", name.into()), ("tile", tile)] {
        let key = string(scope, field);
        if !cb::catching(scope, |s| object.create_data_property(s, key.into(), value))? {
            return Err(cb::type_error(
                scope,
                "walk destinations: could not set a property",
            ));
        }
    }
    Ok(object.into())
}

fn materialize<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, Value<'s>> {
    let shared = shared(scope)?;
    if let Some(values) = shared
        .borrow()
        .values
        .as_ref()
        .map(|values| v8::Local::new(scope, values))
    {
        return Ok(values.into());
    }

    let constructor: Value<'s> = args.get(0);
    if v8::Local::<v8::Function>::try_from(constructor).is_err() {
        return Err(cb::type_error(
            scope,
            "walk destinations: Tile must be a constructor",
        ));
    }

    let destinations = script::native::walk::walk_destinations();
    let mut rows = Vec::with_capacity(destinations.len());
    let mut options = Vec::with_capacity(destinations.len());
    for destination in destinations {
        let name = string(scope, destination.name);
        let tile_args = [
            integer(scope, destination.x),
            integer(scope, destination.z),
            integer(scope, destination.level),
        ];
        let tile = cb::construct(scope, constructor, &tile_args, "Tile")?;
        rows.push(row(scope, name, tile)?);
        options.push(name.into());
    }

    let rows_array = v8::Array::new(scope, rows.len() as i32);
    for (index, value) in rows.into_iter().enumerate() {
        if !cb::catching(scope, |s| rows_array.set_index(s, index as u32, value))? {
            return Err(cb::type_error(
                scope,
                "walk destinations: could not append a row",
            ));
        }
    }
    let options_array = v8::Array::new(scope, options.len() as i32);
    for (index, value) in options.into_iter().enumerate() {
        if !cb::catching(scope, |s| options_array.set_index(s, index as u32, value))? {
            return Err(cb::type_error(
                scope,
                "walk destinations: could not append an option",
            ));
        }
    }
    let values = v8::Array::new(scope, 2);
    for (index, value) in [rows_array.into(), options_array.into()]
        .into_iter()
        .enumerate()
    {
        if !cb::catching(scope, |s| values.set_index(s, index as u32, value))? {
            return Err(cb::type_error(
                scope,
                "walk destinations: could not return the catalog",
            ));
        }
    }
    shared.borrow_mut().values = Some(v8::Global::new(scope, values));
    Ok(values.into())
}

#[cfg(test)]
mod tests {
    include!("walk_destinations_v8_tests.rs");
}
