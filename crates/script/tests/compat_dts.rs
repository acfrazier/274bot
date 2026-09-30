// Compat (JS API v1) declarations — generated from the shim name maps.
use script::compat_dts::{
    collect_compat_surface, collect_compat_surface_from, compat_dts_path, find_export,
    member_names, parse_module_source, render_compat_dts, render_compat_dts_with,
    render_compat_surface, shim_source_pairs, write_compat_dts, CompatExport, DtsExtension,
    FnParam, Member, MemberKind,
};

#[test]
fn compat_dts_is_fresh() {
    let path = compat_dts_path();
    let on_disk =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let rendered = render_compat_dts();
    assert_eq!(
        on_disk, rendered,
        "compat-js/index.d.ts is stale; run: cargo test -p script --test compat_dts regen_compat_dts -- --ignored"
    );
}

#[test]
fn game_shim_methods_are_declared() {
    let src = include_str!("../src/shim/game.js");
    let parsed = parse_module_source("/rs2b0t/bot/api/game/Game.js", src);
    let game = parsed
        .exports
        .iter()
        .find(|e| matches!(e, CompatExport::Object { name, .. } if name == "Game"))
        .expect("Game object");
    let names = member_names(game);
    for need in [
        "ingame",
        "tile",
        "tick",
        "setCombatStyle",
        "openSideTab",
        "teleport",
        "castOnItem",
        "castOnNpc",
    ] {
        assert!(
            names.contains(&need),
            "Game.{need} missing from parsed shim: {names:?}"
        );
    }
    let rendered = render_compat_dts();
    for need in ["ingame", "tile", "setCombatStyle", "teleport", "castOnNpc"] {
        assert!(
            rendered.contains(&format!("{need}(")),
            "generated d.ts missing Game.{need}"
        );
    }
}

#[test]
fn tile_and_bots_come_from_shim_and_prelude() {
    let surface = collect_compat_surface();
    let tile = find_export(&surface, "Tile").expect("Tile");
    match tile {
        CompatExport::Class {
            members, default, ..
        } => {
            assert!(*default, "tile.js default-exports Tile");
            let names = member_names(tile);
            assert!(names.contains(&"distanceTo"), "{names:?}");
            assert!(names.contains(&"from"), "{names:?}");
            let from = members.iter().find(|m| m.name == "from").unwrap();
            assert!(from.is_static);
        }
        other => panic!("Tile should be a class: {other:?}"),
    }
    let looping = find_export(&surface, "LoopingBot").expect("LoopingBot");
    let names = member_names(looping);
    for need in ["onStart", "onStop", "loop", "log", "settings"] {
        assert!(
            names.contains(&need),
            "LoopingBot.{need} missing: {names:?}"
        );
    }
    let settings = match looping {
        CompatExport::Class { members, .. } => members.iter().find(|m| m.name == "settings"),
        _ => None,
    }
    .expect("settings");
    assert_eq!(settings.kind, MemberKind::Getter);
}

/// Adding a method to a shim source without regenerating must change the
/// rendered declarations — the freshness check then fails against on-disk.
#[test]
fn drift_gate_fails_when_shim_gains_a_method() {
    let owned = shim_source_pairs();
    let mut patched_game = None;
    for (spec, src) in &owned {
        if spec.ends_with("/api/game/Game.js") {
            let needle = "async castOnNpc() {
            throw notImpl('Game.castOnNpc');
        },";
            assert!(src.contains(needle), "Game.js castOnNpc needle moved");
            patched_game = Some(src.replace(
                needle,
                "async castOnNpc() {
            throw notImpl('Game.castOnNpc');
        },
        extraProbeMethod() { return 1; },",
            ));
        }
    }
    let patched_game = patched_game.expect("Game.js in shim_modules");
    let pairs: Vec<(&str, &str)> = owned
        .iter()
        .map(|(s, src)| {
            if s.ends_with("/api/game/Game.js") {
                (s.as_str(), patched_game.as_str())
            } else {
                (s.as_str(), src.as_str())
            }
        })
        .collect();
    let original = render_compat_dts();
    let next = render_compat_surface(&collect_compat_surface_from(&pairs));
    assert!(
        next.contains("extraProbeMethod"),
        "generator must pick up a method added to the Game shim"
    );
    assert!(
        !original.contains("extraProbeMethod"),
        "unpatched surface must not already declare extraProbeMethod"
    );
    assert_ne!(
        original, next,
        "adding a shim method must change the generated d.ts so freshness fails"
    );
}

#[test]
fn dts_extension_appends_without_forking_the_generator() {
    let extra = DtsExtension {
        specifier: "/rs2b0t/bot/api/gather/Gatherer.js".to_string(),
        exports: vec![CompatExport::Object {
            name: "Gatherer".to_string(),
            members: vec![Member {
                name: "start".to_string(),
                kind: MemberKind::Method,
                params: vec![FnParam {
                    name: "mode".to_string(),
                    optional: false,
                    rest: false,
                }],
                is_async: true,
                is_static: false,
            }],
        }],
    };
    let src = render_compat_dts_with(&[extra]);
    assert!(src.contains("declare module '*api/gather/Gatherer.js'"));
    assert!(src.contains("start(mode: any): Promise<any>"));
}

#[test]
fn equipment_async_and_inventory_shape() {
    let eq = parse_module_source(
        "/rs2b0t/bot/api/equipment/Equipment.js",
        include_str!("../src/shim/equipment.js"),
    );
    let equipment = eq
        .exports
        .iter()
        .find(|e| matches!(e, CompatExport::Object { name, .. } if name == "Equipment"))
        .expect("Equipment");
    let names = member_names(equipment);
    assert!(names.contains(&"equip"), "{names:?}");
    if let CompatExport::Object { members, .. } = equipment {
        let equip = members.iter().find(|m| m.name == "equip").unwrap();
        assert!(equip.is_async);
    }
    let inv = parse_module_source(
        "/rs2b0t/bot/api/inventory/Inventory.js",
        include_str!("../src/shim/inventory.js"),
    );
    let inventory = inv
        .exports
        .iter()
        .find(|e| matches!(e, CompatExport::Object { name, .. } if name == "Inventory"))
        .expect("Inventory");
    let names = member_names(inventory);
    for need in ["count", "first", "items", "isFull"] {
        assert!(names.contains(&need), "{names:?}");
    }
}

/// Writes `compat-js/index.d.ts` from the shim name maps.
#[test]
#[ignore]
fn regen_compat_dts() {
    write_compat_dts().expect("write compat-js/index.d.ts");
    eprintln!("wrote {}", compat_dts_path().display());
}

/// `COMPAT_DTS_TREE=/tmp/bot cargo test -p script --test compat_dts write_compat_dts_tree_to_env -- --ignored`
#[test]
#[ignore]
fn write_compat_dts_tree_to_env() {
    let dir = std::env::var("COMPAT_DTS_TREE").expect("COMPAT_DTS_TREE");
    script::compat_dts::write_compat_dts_tree(std::path::Path::new(&dir))
        .expect("write compat dts tree");
    eprintln!("wrote tree {}", dir);
}

#[test]
fn dynamic_proxy_bags_are_any() {
    let rendered = render_compat_dts();
    assert!(
        rendered.contains("export const SHOP_DB: any"),
        "SHOP_DB is a dynamic Proxy bag"
    );
    assert!(
        rendered.contains("export const DROP_DB: any"),
        "DROP_DB is a dynamic Proxy bag"
    );
    assert!(
        rendered.contains("export const ITEM_DB: any"),
        "ITEM_DB must not be dropped because the Proxy target is an ident"
    );
    assert!(
        rendered.contains("export const HERBS: any"),
        "HERBS must not be dropped because the Proxy target is an array"
    );
}

#[test]
fn reexports_defaults_and_tile_fields_follow_the_shim() {
    let surface = collect_compat_surface();
    let rendered = render_compat_surface(&surface);

    let style = surface
        .modules
        .iter()
        .find(|m| m.specifier.ends_with("/combat/CombatStyleLogic.js"))
        .expect("CombatStyleLogic");
    assert!(
        style.exports.iter().any(|e| match e {
            CompatExport::Class { name, .. }
            | CompatExport::Object { name, .. }
            | CompatExport::Function { name, .. }
            | CompatExport::Value { name } => name == "SPELL_DB",
        }),
        "import-then-export SPELL_DB must appear"
    );

    let nav = surface
        .modules
        .iter()
        .find(|m| m.specifier.ends_with("/webwalk/Navigator.js"))
        .expect("Navigator");
    assert_eq!(nav.default_export.as_deref(), Some("Navigator"));
    assert!(rendered.contains("export default Navigator"));

    let tile = find_export(&surface, "Tile").expect("Tile");
    let names = member_names(tile);
    for need in ["x", "z", "level"] {
        assert!(
            names.contains(&need),
            "Tile.{need} from constructor this-assign: {names:?}"
        );
    }

    let cook = surface
        .modules
        .iter()
        .find(|m| m.specifier.ends_with("/cooking/CookLocations.js"))
        .expect("CookLocations");
    let cook_names: Vec<_> = cook
        .exports
        .iter()
        .map(|e| match e {
            CompatExport::Class { name, .. }
            | CompatExport::Object { name, .. }
            | CompatExport::Function { name, .. }
            | CompatExport::Value { name } => name.as_str(),
        })
        .collect();
    assert!(cook_names.contains(&"CUSTOM_LOCATION"), "{cook_names:?}");
    assert!(cook_names.contains(&"COOK_LOCATIONS"), "{cook_names:?}");

    let combat = surface
        .modules
        .iter()
        .find(|m| m.specifier.ends_with("/hunting/combat.js"))
        .expect("hunting/combat");
    assert!(
        combat
            .privates
            .iter()
            .any(|e| matches!(e, CompatExport::Class { name, .. } if name == "HuntTask")),
        "unexported HuntTask must be emitted so `extends HuntTask` typechecks"
    );
    assert!(
        rendered.contains("class HuntTask"),
        "HuntTask class in d.ts"
    );
    assert!(
        rendered.contains("want?: any, _walkable?: any, _canStep?: any, directions?: any"),
        "JS optional param must force following params optional: findBurnLane"
    );
}

#[test]
fn dts_tree_mirrors_shim_urls_for_relative_imports() {
    let dir = std::env::temp_dir().join("compat-dts-tree-test");
    let _ = std::fs::remove_dir_all(&dir);
    script::compat_dts::write_compat_dts_tree(&dir).expect("write tree");
    let game = std::fs::read_to_string(dir.join("api/game/Game.d.ts")).expect("Game.d.ts");
    assert!(game.contains("ingame("), "{game}");
    assert!(game.contains("castOnNpc("), "{game}");
    let tile = std::fs::read_to_string(dir.join("geometry/Tile.d.ts")).expect("Tile.d.ts");
    assert!(tile.contains("export default Tile"), "{tile}");
    assert!(tile.contains("x: any"), "{tile}");
    let _ = std::fs::remove_dir_all(&dir);
}
