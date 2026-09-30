// Compat (JS API v1) declarations — authored types gated against the shim.
use script::compat_dts::{
    check_compat_dts_drift, collect_compat_surface, collect_compat_surface_from, compat_dts_path,
    drift_against_shim, find_export, load_authored_dts, load_authored_dts_from, member_names,
    parse_module_source, shim_source_pairs, write_authored_dts_tree, CompatExport, MemberKind,
};
use std::fs;
use std::path::Path;

#[test]
fn compat_dts_is_fresh() {
    check_compat_dts_drift().unwrap_or_else(|e| panic!("{e}"));
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
    let dts = fs::read_to_string(compat_dts_path()).unwrap();
    for need in ["ingame", "tile", "setCombatStyle", "teleport", "castOnNpc"] {
        assert!(
            dts.contains(&format!("{need}(")),
            "authored d.ts missing Game.{need}"
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

/// Adding a method to a shim source must fail the structural drift gate
/// against the authored declarations.
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
    let authored = load_authored_dts().expect("authored d.ts");
    let drifts = drift_against_shim(&authored, &collect_compat_surface_from(&pairs));
    assert!(
        drifts.iter().any(|d| d.contains("extraProbeMethod")),
        "injected Game.extraProbeMethod must fail the drift gate: {drifts:?}"
    );
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

/// Writes a declaration tree for `tsc` from the authored file.
/// `COMPAT_DTS_TREE=/tmp/bot cargo test -p script --test compat_dts write_compat_dts_tree_to_env -- --ignored`
#[test]
#[ignore]
fn write_compat_dts_tree_to_env() {
    let dir = std::env::var("COMPAT_DTS_TREE").expect("COMPAT_DTS_TREE");
    script::compat_dts::write_compat_dts_tree(Path::new(&dir)).expect("write compat dts tree");
    eprintln!("wrote tree {dir}");
}

#[test]
fn barrel_game_is_a_typed_reexport() {
    let authored = load_authored_dts().expect("authored d.ts");
    let barrel = authored
        .modules
        .iter()
        .find(|m| m.specifier == "@rs2b0t/api")
        .expect("@rs2b0t/api barrel");
    let game = barrel.exports.iter().find(|e| match e {
        CompatExport::Class { name, .. }
        | CompatExport::Object { name, .. }
        | CompatExport::Function { name, .. }
        | CompatExport::Value { name } => name == "Game",
    });
    match game {
        Some(CompatExport::Value { .. }) => {}
        other => panic!("barrel Game must be a typed re-export, got {other:?}"),
    }
    let names: Vec<&str> = barrel
        .exports
        .iter()
        .map(|e| match e {
            CompatExport::Class { name, .. }
            | CompatExport::Object { name, .. }
            | CompatExport::Function { name, .. }
            | CompatExport::Value { name } => name.as_str(),
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    for name in &names {
        assert!(seen.insert(*name), "duplicate barrel export {name}");
    }
}

/// Adding a member to the authored Game declaration must fail the reverse drift pass.
#[test]
fn drift_gate_fails_when_declaration_gains_a_member() {
    let src = fs::read_to_string(compat_dts_path()).unwrap();
    let needle = "    sceneReady(): boolean;";
    assert!(src.contains(needle), "Game.sceneReady needle moved");
    let patched = src.replace(
        needle,
        "    sceneReady(): boolean;\n    reviewGhostMember(): void;",
    );
    let dir = std::env::temp_dir().join("compat-dts-extra-member");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("index.d.ts"), patched).unwrap();
    let authored = load_authored_dts_from(&dir).expect("load patched d.ts");
    let drifts = drift_against_shim(&authored, &collect_compat_surface());
    assert!(
        drifts.iter().any(|d| d.contains("reviewGhostMember")),
        "injected Game.reviewGhostMember must fail the drift gate: {drifts:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reexports_defaults_and_tile_fields_follow_the_shim() {
    let surface = collect_compat_surface();
    let dts = fs::read_to_string(compat_dts_path()).unwrap();

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
    assert!(
        dts.contains("export default Navigator"),
        "authored Navigator default export"
    );

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
    assert!(dts.contains("class HuntTask"), "HuntTask class in d.ts");
    assert!(
        dts.contains("want?:") && dts.contains("directions?:"),
        "JS optional param must force following params optional: findBurnLane"
    );
}

#[test]
fn dts_tree_mirrors_shim_urls_for_relative_imports() {
    let dir = std::env::temp_dir().join("compat-dts-tree-test");
    let _ = fs::remove_dir_all(&dir);
    script::compat_dts::write_compat_dts_tree(&dir).expect("write tree");
    assert!(
        dir.join("api/game/Game.d.ts").is_file(),
        "tree must emit Game.d.ts"
    );
    assert!(
        dir.join("geometry/Tile.d.ts").is_file(),
        "tree must emit Tile.d.ts"
    );
    assert!(
        dir.join("runtime/BotHost.d.ts").is_file(),
        "tree must emit BotHost.d.ts"
    );
    let npcs = fs::read_to_string(dir.join("api/npcs/Npcs.d.ts")).expect("Npcs.d.ts");
    assert!(
        npcs.contains("from '../../geometry/Tile.js'"),
        "entity tree files must rewrite *geometry/Tile.js to a relative import: {npcs}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn bot_host_shorthand_add_tick_listener_is_a_member() {
    let parsed = parse_module_source(
        "/rs2b0t/bot/runtime/BotHost.js",
        include_str!("../src/shim/bot_host.js"),
    );
    let host = parsed
        .exports
        .iter()
        .find(|e| matches!(e, CompatExport::Object { name, .. } if name == "BotHost"))
        .expect("BotHost");
    let names = member_names(host);
    assert!(
        names.contains(&"addTickListener"),
        "shorthand addTickListener must be a BotHost member: {names:?}"
    );
    if let CompatExport::Object { members, .. } = host {
        let add = members
            .iter()
            .find(|m| m.name == "addTickListener")
            .unwrap();
        assert_eq!(add.kind, MemberKind::Method);
        assert_eq!(add.params.len(), 1);
    }
    let dts = fs::read_to_string(compat_dts_path()).unwrap();
    assert!(
        dts.contains("addTickListener"),
        "authored d.ts must declare BotHost.addTickListener"
    );
}

#[test]
fn bank_deposit_and_delay_until_are_promise_returning() {
    let bank = parse_module_source(
        "/rs2b0t/bot/api/bank/Bank.js",
        include_str!("../src/shim/bank.js"),
    );
    let bank_obj = bank
        .exports
        .iter()
        .find(|e| matches!(e, CompatExport::Object { name, .. } if name == "Bank"))
        .expect("Bank");
    if let CompatExport::Object { members, .. } = bank_obj {
        let deposit = members.iter().find(|m| m.name == "deposit").unwrap();
        assert!(
            deposit.is_async,
            "Bank.deposit returns bankOp() which is async"
        );
        let withdraw = members.iter().find(|m| m.name == "withdraw").unwrap();
        assert!(withdraw.is_async, "Bank.withdraw returns bankOp()");
    }
    let exec = parse_module_source(
        "/rs2b0t/bot/api/execution/Execution.js",
        include_str!("../src/shim/execution.js"),
    );
    let execution = exec
        .exports
        .iter()
        .find(|e| matches!(e, CompatExport::Object { name, .. } if name == "Execution"))
        .expect("Execution");
    if let CompatExport::Object { members, .. } = execution {
        let delay_until = members.iter().find(|m| m.name == "delayUntil").unwrap();
        assert!(
            delay_until.is_async,
            "Execution.delayUntil returns park.enqueue() / Promise"
        );
    }
}

/// Consumer TypeScript probe: good uses typecheck, bad `Game` uses must not.
/// Also type-checks `compat-js/index.d.ts` with `skipLibCheck: false`.
/// Requires `npx`; always runs the pinned TypeScript 5.8.3 (a `tsc` on `PATH` may be 7.x). Run:
/// `npx -p typescript@5.8.3 --yes tsc --noEmit -p crates/script/tests/compat_dts_probe`
#[test]
#[ignore = "requires tsc; npx -p typescript@5.8.3 --yes tsc --noEmit -p crates/script/tests/compat_dts_probe"]
fn tsc_consumer_probe_rejects_wrong_uses() {
    let probe = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compat_dts_probe");
    let mut cmd = tsc_command();
    cmd.arg("--noEmit").arg("-p").arg(&probe);
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("tsc probe failed to spawn: {e}"));
    if !out.status.success() {
        panic!(
            "tsc consumer probe failed (skipLibCheck: false):\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// Frozen-valid consumer from the R4 review (no `@ts-expect-error`). Must be 0 diagnostics
/// against the authored declarations under pinned TypeScript 5.8.3.
/// `npx -p typescript@5.8.3 --yes tsc --noEmit -p crates/script/tests/compat_dts_probe/consumer.tsconfig.json`
#[test]
#[ignore = "requires tsc; npx -p typescript@5.8.3 --yes tsc --noEmit -p crates/script/tests/compat_dts_probe/consumer.tsconfig.json"]
fn tsc_valid_consumer_probe_has_zero_diagnostics() {
    let probe =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compat_dts_probe/consumer.tsconfig.json");
    let mut cmd = tsc_command();
    cmd.arg("--noEmit").arg("-p").arg(&probe);
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("tsc valid consumer probe failed to spawn: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}\n{stderr}");
    let diag_count = combined.lines().filter(|l| l.contains("error TS")).count();
    if !out.status.success() || diag_count != 0 {
        panic!(
            "valid consumer must have 0 diagnostics against authored declarations (saw {diag_count}):\n{stdout}\n{stderr}"
        );
    }
}

/// Pinned compiler: a bare `tsc` on `PATH` (Homebrew ships 7.x) would change the diagnostics.
fn tsc_command() -> std::process::Command {
    let mut npx = std::process::Command::new("npx");
    npx.args(["-p", "typescript@5.8.3", "--yes", "tsc"]);
    npx
}

fn write_gatherer_extension(dir: &Path, with_barrel: bool, with_file: bool) {
    let src = fs::read_to_string(compat_dts_path()).unwrap();
    fs::create_dir_all(dir).unwrap();
    let barrel = if with_barrel {
        src.replacen(
            "declare module '@rs2b0t/api' {",
            "declare module '@rs2b0t/api' {\n  export { Gatherer } from '*api/gather/Gatherer.js';",
            1,
        )
    } else {
        src
    };
    fs::write(dir.join("index.d.ts"), barrel).unwrap();
    if with_file {
        let gather_dir = dir.join("api/gather");
        fs::create_dir_all(&gather_dir).unwrap();
        fs::write(
            gather_dir.join("Gatherer.d.ts"),
            r#"/** Native gatherer start — O-SCRIPT-API extension example. */
export type GatherMode = 'woodcut' | 'mine' | 'fish' | 'harvest';
export interface GatherStart {
  ok: boolean;
  site: string;
}
export const Gatherer: {
  start(mode: GatherMode): Promise<GatherStart>;
};
"#,
        )
        .unwrap();
    }
}

#[test]
fn extension_accepted_when_typed_module_and_barrel_exist() {
    let dir = std::env::temp_dir().join("compat-dts-ext-ok");
    let _ = fs::remove_dir_all(&dir);
    write_gatherer_extension(&dir, true, true);
    let authored = load_authored_dts_from(&dir).expect("load extension dir");
    let shim = collect_compat_surface();
    let drifts = drift_against_shim(&authored, &shim);
    assert!(
        drifts.is_empty(),
        "typed Gatherer module + barrel export must be accepted: {drifts:?}"
    );
    let gatherer = authored
        .modules
        .iter()
        .find(|m| m.specifier.contains("gather/Gatherer"))
        .expect("Gatherer module");
    let start = gatherer
        .exports
        .iter()
        .find(|e| matches!(e, CompatExport::Object { name, .. } if name == "Gatherer"));
    match start {
        Some(CompatExport::Object { members, .. }) => {
            let m = members.iter().find(|x| x.name == "start").expect("start");
            assert!(m.is_async, "Gatherer.start is Promise<GatherStart>");
            assert_eq!(m.params.len(), 1);
            assert_eq!(m.params[0].name, "mode");
        }
        other => panic!("Gatherer object: {other:?}"),
    }
    let tree = std::env::temp_dir().join("compat-dts-ext-tree");
    let _ = fs::remove_dir_all(&tree);
    write_authored_dts_tree(&authored, &tree).expect("tree");
    let file = fs::read_to_string(tree.join("api/gather/Gatherer.d.ts")).expect("Gatherer.d.ts");
    assert!(
        file.contains("start(mode: GatherMode): Promise<GatherStart>"),
        "{file}"
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&tree);
}

#[test]
fn extension_rejected_without_barrel_export() {
    let dir = std::env::temp_dir().join("compat-dts-ext-nobarrel");
    let _ = fs::remove_dir_all(&dir);
    write_gatherer_extension(&dir, false, true);
    let authored = load_authored_dts_from(&dir).expect("load");
    let drifts = drift_against_shim(&authored, &collect_compat_surface());
    assert!(
        drifts
            .iter()
            .any(|d| d.contains("Gatherer") && d.contains("barrel")),
        "missing barrel export must fail: {drifts:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn extension_rejected_without_module_file() {
    let dir = std::env::temp_dir().join("compat-dts-ext-nofile");
    let _ = fs::remove_dir_all(&dir);
    write_gatherer_extension(&dir, true, false);
    let authored = load_authored_dts_from(&dir).expect("load");
    let has_gatherer = authored
        .modules
        .iter()
        .any(|m| m.specifier.contains("gather/Gatherer"));
    assert!(
        !has_gatherer,
        "barrel-only Gatherer without a module file must not invent a module"
    );
    let _ = fs::remove_dir_all(&dir);
}
