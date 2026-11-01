//! Native V8 materialization for the fixed eight-tile step-off candidate set.
use super::callback_v8;
use api::WorldTile;
use rustyscript::Runtime;

const FIELDS: [&str; 3] = ["x", "z", "level"];

pub(crate) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let mut scope = runtime.deno_runtime().handle_scope();
    let shape = v8::ObjectTemplate::new(&mut scope);
    let placeholder: v8::Local<v8::Data> = v8::undefined(&mut scope).into();
    for field in FIELDS {
        let key = v8::String::new(&mut scope, field)
            .ok_or_else(|| format!("step-off candidate shape: {field}"))?;
        shape.set(key.into(), placeholder);
    }
    let shape: v8::Local<v8::Data> = shape.into();
    let callback = v8::Function::builder(step_off_candidates)
        .data(shape)
        .build(&mut scope)
        .ok_or_else(|| "step-off candidates: callback".to_string())?;
    let context = runtime.deno_runtime().main_context();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_step_off_candidates")
        .ok_or_else(|| "step-off candidates: name".to_string())?;
    global
        .set(&mut scope, name.into(), callback.into())
        .then_some(())
        .ok_or_else(|| "step-off candidates: global".to_string())
}

fn int_arg<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    name: &str,
) -> callback_v8::JsResult<'s, i32> {
    let value = args.get(index);
    let Some(number) = value.number_value(scope) else {
        return Err(callback_v8::type_error(scope, &format!("stepOffCandidates: invalid {name}")));
    };
    if !number.is_finite() || number.fract() != 0.0 || number < f64::from(i32::MIN) || number > f64::from(i32::MAX) {
        return Err(callback_v8::type_error(scope, &format!("stepOffCandidates: invalid {name}")));
    }
    Ok(number as i32)
}

fn step_off_candidates<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let x = int_arg(scope, &args, 0, "origin x")?;
        let z = int_arg(scope, &args, 1, "origin z")?;
        let level = int_arg(scope, &args, 2, "origin level")?;
        let away_x = int_arg(scope, &args, 3, "threat x")?;
        let away_z = int_arg(scope, &args, 4, "threat z")?;
        let template = v8::Local::<v8::ObjectTemplate>::try_from(args.data())
            .map_err(|_| callback_v8::type_error(scope, "stepOffCandidates: invalid row shape"))?;
        let candidates = script::native::walk::step_off_candidates(
            WorldTile { x, z, level },
            (away_x, away_z),
        );
        let rows = v8::Array::new(scope, candidates.len() as i32);
        for (index, tile) in candidates.into_iter().enumerate() {
            let row = template
                .new_instance(scope)
                .ok_or_else(|| callback_v8::type_error(scope, "stepOffCandidates: row allocation"))?;
            for (field, value) in FIELDS.into_iter().zip([tile.x, tile.z, tile.level]) {
                let key = v8::String::new(scope, field)
                    .ok_or_else(|| callback_v8::type_error(scope, "stepOffCandidates: key allocation"))?;
                let value = v8::Integer::new(scope, value).into();
                if !row.create_data_property(scope, key.into(), value)? {
                    return Err(callback_v8::type_error(scope, "stepOffCandidates: row property"));
                }
            }
            if !rows.create_data_property(scope, index as u32, row.into())? {
                return Err(callback_v8::type_error(scope, "stepOffCandidates: array row"));
            }
        }
        Ok(rows.into())
    })();
    callback_v8::finish(scope, rv, result);
}

#[cfg(test)]
#[path = "step_off_candidates_v8_tests.rs"]
mod tests;
