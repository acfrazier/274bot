// Declared JS catalog ABI (rs2b0t-api index.d.ts names). Not the Rust host ABI.
use script::declared_abi::{
    fixture_path, load_fixture, parse_index_dts, write_declared_surface, write_fixture,
    DeclaredKind,
};

const SLICE: &str = r#"
export const apiVersion: number;
export type MeleeCombatStyle = 'attack' | 'strength';
export const Game: {
    ingame(): boolean;
    setCombatStyle(style: MeleeCombatStyle): boolean;
    /**
     * @internal
     */
    bindLog(): void;
};
export class Tile {
    distanceTo(other: WorldTile): number;
    static from(tile: WorldTile): Tile;
}
export function withdrawOp(ops: readonly string[], which: string): string | null;
"#;

#[test]
fn parse_index_dts_names_only_skips_types_and_internal() {
    let got = parse_index_dts(SLICE);
    let names: Vec<_> = got.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"apiVersion"));
    assert!(names.contains(&"Game"));
    assert!(names.contains(&"Tile"));
    assert!(names.contains(&"withdrawOp"));
    assert!(!names.iter().any(|n| *n == "MeleeCombatStyle"));
    let game = got.iter().find(|e| e.name == "Game").unwrap();
    assert_eq!(game.kind, DeclaredKind::Object);
    assert!(game.members.iter().any(|m| m == "ingame"));
    assert!(game.members.iter().any(|m| m == "setCombatStyle"));
    assert!(!game.members.iter().any(|m| m == "bindLog"));
    let tile = got.iter().find(|e| e.name == "Tile").unwrap();
    assert_eq!(tile.kind, DeclaredKind::Class);
    assert!(tile.members.iter().any(|m| m == "distanceTo"));
    assert!(tile.members.iter().any(|m| m == "from"));
    let api = got.iter().find(|e| e.name == "apiVersion").unwrap();
    assert_eq!(api.kind, DeclaredKind::Value);
    assert!(api.members.is_empty());
    let fun = got.iter().find(|e| e.name == "withdrawOp").unwrap();
    assert_eq!(fun.kind, DeclaredKind::Function);
}

/// Every fixture export is on `@rs2b0t/api` and every listed member is
/// present (`in` / typeof function). Missing members that throw `not v1`
/// from the Proxy get-trap fail this test; calling a stub is not required.
#[test]
fn declared_abi_members_are_loadable() {
    use script::load::{LoadIsolate, LoadShape};

    let fixture = load_fixture().expect("js_declared_abi.json");
    let mut body = String::from(
        r#"
import * as api from '@rs2b0t/api';
export default class T extends LoopingBot {
    loop() {
        const missing = [];
        const checkExport = (name) => {
            try {
                if (!(name in api) || api[name] === undefined) {
                    missing.push(name + ' missing export');
                    return null;
                }
            } catch (e) {
                missing.push(name + ' export throw ' + String(e));
                return null;
            }
            return api[name];
        };
        const checkMember = (ns, obj, member) => {
            try {
                if (obj == null) return;
                if (typeof obj === 'function') {
                    if (member === 'constructor' || member in obj) return;
                    if (obj.prototype && member in obj.prototype) return;
                    try {
                        const inst = new obj(0, 0, 0);
                        if (member in inst) return;
                    } catch (_) {}
                    try {
                        const Dummy = class extends obj { loop() {} };
                        if (member in new Dummy()) return;
                    } catch (_) {}
                    missing.push(ns + '.' + member + ' missing on class');
                    return;
                }
                if (member in obj) return;
                missing.push(ns + '.' + member + ' missing');
            } catch (e) {
                missing.push(ns + '.' + member + ' ' + String(e));
            }
        };
"#,
    );
    for exp in &fixture {
        let name = &exp.name;
        body.push_str(&format!("        const _{name} = checkExport({name:?});\n"));
        match exp.kind {
            DeclaredKind::Value => {}
            DeclaredKind::Function => {
                body.push_str(&format!(
                    "        if (_{name} != null && typeof _{name} !== 'function') missing.push({name:?} + ' not a function');\n"
                ));
            }
            DeclaredKind::Object | DeclaredKind::Class => {
                for m in &exp.members {
                    body.push_str(&format!(
                        "        checkMember({name:?}, _{name}, {m:?});\n"
                    ));
                }
            }
        }
    }
    body.push_str(
        r#"
        globalThis.__abi_missing = missing;
    }
}
"#,
    );

    let iso = LoadIsolate::spawn(body, LoadShape::CompatClass, vec![])
        .expect("declared ABI probe script must load");
    iso.on_game_tick(1);
    let missing = iso.probe("__abi_missing").expect("probe __abi_missing");
    iso.join();
    let arr = missing.as_array().cloned().unwrap_or_default();
    let lines: Vec<String> = arr
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    assert!(
        lines.is_empty(),
        "declared ABI members not loadable ({}):\n{}",
        lines.len(),
        lines.join("\n")
    );
}

fn catalog_index_dts(root: &std::path::Path) -> std::path::PathBuf {
    root.join("packages/rs2b0t-api/index.d.ts")
}

/// When `$RS2B0T` (or the persisted catalog root) is set, the checked-in
/// fixture must match a fresh parse of that tree's `index.d.ts`.
#[test]
fn declared_abi_fixture_matches_local_dts() {
    let Some(root) = script::rs2b0t_root() else {
        return;
    };
    let path = catalog_index_dts(&root);
    let Ok(src) = std::fs::read_to_string(&path) else {
        return;
    };
    let live = parse_index_dts(&src);
    let fixture = load_fixture().expect("js_declared_abi.json");
    assert_eq!(
        live, fixture,
        "js_declared_abi.json is stale; run: cargo test -p script --test declared_abi regen_js_declared_abi -- --ignored"
    );
}

/// Writes `tests/fixtures/js_declared_abi.json` from the local catalog.
/// Fail-closed: does not write an empty list when the catalog is missing.
#[test]
#[ignore]
fn regen_js_declared_abi() {
    let root = script::rs2b0t_root().expect("regen needs $RS2B0T (or persisted rs2b0t-path)");
    let path = catalog_index_dts(&root);
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let exports = parse_index_dts(&src);
    assert!(
        !exports.is_empty(),
        "parsed zero exports from {} — refusing to write an empty fixture",
        path.display()
    );
    write_fixture(&exports).expect("write js_declared_abi.json");
    write_declared_surface(&exports).expect("write declared_surface.js");
    eprintln!(
        "wrote {} exports to {} and {}",
        exports.len(),
        fixture_path().display(),
        script::declared_abi::declared_surface_path().display()
    );
}

