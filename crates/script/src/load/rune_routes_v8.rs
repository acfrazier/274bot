//! Materialize the runecrafting compatibility catalog from the Rust route roster.
use super::callback_v8;
use crate::content::RUNE_ROUTES;
use rustyscript::deno_core::v8;
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    callback_v8::install(runtime, "__rs2b0t_rune_craft_locations", catalog)
}

fn catalog<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = materialize(scope, args.get(0));
    callback_v8::finish(scope, rv, result);
}

fn materialize<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile_constructor: v8::Local<'s, v8::Value>,
) -> callback_v8::JsResult<'s, v8::Local<'s, v8::Value>> {
    let runes = v8::Object::new(scope);
    let options = v8::Array::new(scope, RUNE_ROUTES.len() as i32);

    for (index, route) in RUNE_ROUTES.iter().enumerate() {
        let tile_args: [v8::Local<'s, v8::Value>; 3] = [
            v8::Integer::new(scope, route.ruins.x).into(),
            v8::Integer::new(scope, route.ruins.z).into(),
            v8::Integer::new(scope, route.ruins.level).into(),
        ];
        let ruins = callback_v8::construct(scope, tile_constructor, &tile_args, "Tile")?;
        let route_value = callback_v8::object(
            scope,
            &[
                ("rune", callback_v8::string(scope, route.rune)),
                ("talisman", callback_v8::string(scope, route.talisman)),
                ("level", v8::Integer::new(scope, route.level).into()),
                ("bank", callback_v8::string(scope, route.bank)),
                ("ruins", ruins),
            ],
        )?;

        let key = callback_v8::string(scope, route.rune);
        if !callback_v8::catching(scope, |s| {
            runes.create_data_property(s, key.cast(), route_value)
        })? {
            return Err(callback_v8::type_error(
                scope,
                "could not materialize runecrafting route",
            ));
        }

        let option = callback_v8::string(scope, route.rune);
        if !callback_v8::catching(scope, |s| options.set_index(s, index as u32, option))? {
            return Err(callback_v8::type_error(
                scope,
                "could not materialize runecrafting options",
            ));
        }
    }

    let default_rune = RUNE_ROUTES
        .first()
        .expect("the runecrafting route roster must contain its default")
        .rune;
    callback_v8::object(
        scope,
        &[
            ("RUNES", runes.into()),
            ("RUNE_OPTIONS", options.into()),
            ("DEFAULT_RUNE", callback_v8::string(scope, default_rune)),
        ],
    )
}
