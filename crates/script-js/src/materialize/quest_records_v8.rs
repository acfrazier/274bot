//! The `QUESTS` array of rs2b0t's `api/ai/quests/data/quests.js`, built from the
//! selected content (`script::quester::compat_quests`).
//!
//! One native call builds the whole array once, when the module is first
//! imported. Every object comes from an `ObjectTemplate` for its own-key list
//! (built once per call), so it has fast properties without a JavaScript path
//! warming it.

use super::callback_v8::{self as cb, JsResult};
use crate::scene::supply_v2;
use rustyscript::Runtime;
use script::quester::compat_quests::{compat_quests, CompatQuest, CompatRequirements};
use std::collections::HashMap;

type Value<'s> = v8::Local<'s, v8::Value>;
type Shapes<'s> = HashMap<Vec<&'static str>, v8::Local<'s, v8::ObjectTemplate>>;

pub(crate) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_quest_records", callback)
}

fn callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    _args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = build(scope);
    cb::finish(scope, rv, result);
}

/// Without selected quest content there are no records. The module is evaluated whenever a card's
/// module graph keeps it, including isolates that run with no content, so this does not throw.
fn build<'s>(scope: &mut v8::HandleScope<'s>) -> JsResult<'s, Value<'s>> {
    let quests = supply_v2::selected_data()
        .and_then(|selected| compat_quests(&selected))
        .unwrap_or_default();
    let mut shapes = Shapes::new();
    let mut rows = Vec::with_capacity(quests.len());
    for quest in &quests {
        rows.push(record(scope, &mut shapes, quest)?);
    }
    Ok(v8::Array::new_with_elements(scope, &rows).into())
}

fn record<'s>(
    scope: &mut v8::HandleScope<'s>,
    shapes: &mut Shapes<'s>,
    quest: &CompatQuest,
) -> JsResult<'s, Value<'s>> {
    let id = cb::string(scope, &quest.id);
    let name = cb::string(scope, &quest.name);
    let points = int(scope, i64::from(quest.quest_points));
    match &quest.requirements {
        Some(requirements) => {
            let requirements = requirements_object(scope, shapes, requirements)?;
            object(
                scope,
                shapes,
                &["id", "name", "questPoints", "requirements"],
                &[id, name, points, requirements],
            )
        }
        None => object(
            scope,
            shapes,
            &["id", "name", "questPoints"],
            &[id, name, points],
        ),
    }
}

/// Only the requirements the Path states, in the order the catalog's type lists them.
fn requirements_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    shapes: &mut Shapes<'s>,
    requirements: &CompatRequirements,
) -> JsResult<'s, Value<'s>> {
    let mut keys = Vec::with_capacity(3);
    let mut values = Vec::with_capacity(3);
    if let Some(points) = requirements.min_quest_points {
        keys.push("minQuestPoints");
        values.push(int(scope, i64::from(points)));
    }
    if !requirements.quests.is_empty() {
        let names: Vec<_> = requirements
            .quests
            .iter()
            .map(|name| cb::string(scope, name))
            .collect();
        keys.push("quests");
        values.push(v8::Array::new_with_elements(scope, &names).into());
    }
    if !requirements.skills.is_empty() {
        let mut gates = Vec::with_capacity(requirements.skills.len());
        for (skill, level) in &requirements.skills {
            let skill = cb::string(scope, skill);
            let level = int(scope, i64::from(*level));
            gates.push(object(scope, shapes, &["skill", "level"], &[skill, level])?);
        }
        keys.push("skills");
        values.push(v8::Array::new_with_elements(scope, &gates).into());
    }
    object(scope, shapes, &keys, &values)
}

fn int<'s>(scope: &mut v8::HandleScope<'s>, value: i64) -> Value<'s> {
    v8::Number::new(scope, value as f64).into()
}

/// A plain object with these own keys in this order, from the template for the key list.
fn object<'s>(
    scope: &mut v8::HandleScope<'s>,
    shapes: &mut Shapes<'s>,
    keys: &[&'static str],
    values: &[Value<'s>],
) -> JsResult<'s, Value<'s>> {
    debug_assert_eq!(keys.len(), values.len());
    let template = match shapes.get(keys) {
        Some(template) => *template,
        None => {
            let template = v8::ObjectTemplate::new(scope);
            let placeholder: v8::Local<v8::Data> = v8::undefined(scope).into();
            for name in keys {
                let name = cb::string(scope, name);
                template.set(name.cast(), placeholder);
            }
            shapes.insert(keys.to_vec(), template);
            template
        }
    };
    let Some(row) = template.new_instance(scope) else {
        return Err(cb::type_error(
            scope,
            "quest records: object allocation failed",
        ));
    };
    for (name, value) in keys.iter().zip(values) {
        let name = cb::string(scope, name);
        if !cb::catching(scope, |s| row.create_data_property(s, name.cast(), *value))? {
            return Err(cb::type_error(
                scope,
                "quest records: could not set a property",
            ));
        }
    }
    Ok(row.into())
}

#[cfg(test)]
#[path = "quest_records_v8_tests.rs"]
mod tests;
