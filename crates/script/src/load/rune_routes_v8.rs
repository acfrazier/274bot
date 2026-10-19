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
    let mut default_rune = None;

    for (index, route) in RUNE_ROUTES.iter().enumerate() {
        let tile_args: [v8::Local<'s, v8::Value>; 3] = [
            v8::Integer::new(scope, route.ruins.x).into(),
            v8::Integer::new(scope, route.ruins.z).into(),
            v8::Integer::new(scope, route.ruins.level).into(),
        ];
        let ruins = callback_v8::construct(scope, tile_constructor, &tile_args, "Tile")?;
        let rune = callback_v8::string(scope, route.rune);
        if index == 0 {
            default_rune = Some(rune);
        }
        let talisman = callback_v8::string(scope, route.talisman);
        let level = v8::Integer::new(scope, route.level).into();
        let bank = callback_v8::string(scope, route.bank);
        let route_value = callback_v8::object(
            scope,
            &[
                ("rune", rune),
                ("talisman", talisman),
                ("level", level),
                ("bank", bank),
                ("ruins", ruins),
            ],
        )?;

        if !callback_v8::catching(scope, |s| {
            runes.create_data_property(s, rune.cast(), route_value)
        })? {
            return Err(callback_v8::type_error(
                scope,
                "could not materialize runecrafting route",
            ));
        }

        if !callback_v8::catching(scope, |s| options.set_index(s, index as u32, rune))? {
            return Err(callback_v8::type_error(
                scope,
                "could not materialize runecrafting options",
            ));
        }
    }

    let default_rune =
        default_rune.expect("the runecrafting route roster must contain its default");
    callback_v8::object(
        scope,
        &[
            ("RUNES", runes.into()),
            ("RUNE_OPTIONS", options.into()),
            ("DEFAULT_RUNE", default_rune),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyscript::{Module, RuntimeOptions};

    fn setup() -> Runtime {
        crate::load::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        callback_v8::install_ops(&mut runtime).unwrap();
        install(&mut runtime).unwrap();
        let tile_source = format!(
            "{}\nglobalThis.RuneTestTile = Tile;\nglobalThis.runeTestCount = {};",
            include_str!("../shim/tile.js"),
            RUNE_ROUTES.len()
        );
        runtime
            .load_module(&Module::new("native-rune-tile.js", tile_source))
            .unwrap();
        runtime
            .eval::<()>(
                r#"
globalThis.checkRuneCatalog = () => {
    const { RUNES, RUNE_OPTIONS, DEFAULT_RUNE } =
        __rs2b0t_rune_craft_locations(RuneTestTile);
    const keys = Object.keys(RUNES);
    return keys.length === runeTestCount &&
        RUNE_OPTIONS.length === keys.length &&
        DEFAULT_RUNE === keys[0] &&
        keys.every((name, index) => {
            const route = RUNES[name];
            return route.rune === name &&
                RUNE_OPTIONS[index] === name &&
                route.ruins instanceof RuneTestTile &&
                Object.getPrototypeOf(route.ruins) === RuneTestTile.prototype;
        });
};
"#,
            )
            .unwrap();
        runtime
    }

    #[test]
    fn rune_catalog_keeps_zero_cold_and_warm_rust_heap() {
        let mut runtime = setup();
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let key = callback_v8::string(&mut scope, "checkRuneCatalog");
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
        println!("Runecrafting production-Tile catalog cold={cold:?}; warm1000={warm:?}");
        assert_eq!(cold, allocation_counter::AllocationInfo::default());
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }
}
