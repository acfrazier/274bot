// Declared JS catalog ABI (rs2b0t-api index.d.ts names). Not the Rust host ABI.
use script::declared_abi::{
    declared_surface_path, fixture_path, load_fixture, parse_index_dts, render_declared_surface,
    write_declared_surface, write_fixture, DeclaredKind,
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
    assert!(!names.contains(&"MeleeCombatStyle"));
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
/// present (`in` / typeof function). Missing members that throw `not impl`
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
                    body.push_str(&format!("        checkMember({name:?}, _{name}, {m:?});\n"));
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

/// Legacy ENT exports and Execution.noteProgress declared by the catalog.
#[test]
fn declared_abi_has_note_progress_and_ent_quartet() {
    let fixture = load_fixture().expect("js_declared_abi.json");
    let execution = fixture
        .iter()
        .find(|e| e.name == "Execution")
        .expect("Execution");
    assert_eq!(execution.kind, DeclaredKind::Object);
    assert!(
        execution.members.iter().any(|m| m == "noteProgress"),
        "Execution.noteProgress must be a declared member"
    );
    for name in [
        "ENT_NPC_IDS",
        "ENT_LIFE_TICKS",
        "isEntNpcId",
        "entNpcOnTile",
    ] {
        assert!(
            fixture.iter().any(|e| e.name == name),
            "missing declared export {name}"
        );
    }
    assert_eq!(
        fixture
            .iter()
            .find(|e| e.name == "ENT_NPC_IDS")
            .unwrap()
            .kind,
        DeclaredKind::Value
    );
    assert_eq!(
        fixture
            .iter()
            .find(|e| e.name == "ENT_LIFE_TICKS")
            .unwrap()
            .kind,
        DeclaredKind::Value
    );
    assert_eq!(
        fixture
            .iter()
            .find(|e| e.name == "isEntNpcId")
            .unwrap()
            .kind,
        DeclaredKind::Function
    );
    assert_eq!(
        fixture
            .iter()
            .find(|e| e.name == "entNpcOnTile")
            .unwrap()
            .kind,
        DeclaredKind::Function
    );
}

/// Writes `tests/fixtures/js_declared_abi.json` from the local catalog.
/// Fail-closed: does not write an empty list when the catalog is missing.
#[test]
#[ignore]
fn regen_js_declared_abi() {
    let root = script::rs2b0t_root().expect("regen needs $RS2B0T (or persisted rs2b0t-path)");
    let path = catalog_index_dts(&root);
    let src =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
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

/// Declared symbols without a shim implementation throw on use instead of
/// returning fake values. An unimplemented callable must name the member.
#[test]
fn declared_stubs_throw_on_use() {
    use script::load::{LoadIsolate, LoadShape};

    let src = r#"
import * as api from '@rs2b0t/api';
import { bankPace } from '../../api/bank/Banking.js';
export default class T extends LoopingBot {
    async loop() {
        const use = async (fn) => {
            try {
                await fn();
                return 'no-throw';
            } catch (e) {
                return String(e.message || e);
            }
        };
        globalThis.__probe = await Promise.all([
            use(() => api.ALL_FISHING_GEAR_NAMES.includes('net')),
            use(() => String(api.COINS)),
            use(() => api.FISHING_LOCATIONS.length),
            use(() => api.MAP_SQUARE + 1),
            use(() => Object.prototype.toString.call(api.WALK_OPTIONS)),
            use(() => api.AXE_SHOP_COSTS.map((c) => c)),
            use(() => bankPace()),
        ]);
}
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![])
        .expect("declared stub probe must load");
    iso.on_game_tick(1);
    let probe = iso.probe("__probe").expect("probe __probe");
    iso.join();
    let rows = probe.as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 7, "every stub use must be probed: {probe:?}");
    for (i, row) in rows.iter().enumerate() {
        let text = row.as_str().unwrap_or("");
        assert!(
            text.contains("not impl"),
            "stub use {i} must throw not impl, got {text:?}"
        );
    }
    let bank_pace = rows[6].as_str().unwrap_or("");
    assert!(
        bank_pace.contains("not impl: bankPace"),
        "unimplemented call must name bankPace, got {bank_pace:?}"
    );
}

/// Every not-impl overlay citation points to a throwing native refusal, and
/// every named member still throws that refusal when imported from Load.
#[test]
fn not_impl_overlay_runtime_members_still_throw() {
    use script::load::{LoadIsolate, LoadShape};

    let entries: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../compat-js/overlay-not-impl.json"))
            .expect("not-impl overlay JSON");
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut modules = Vec::<String>::new();
    for entry in &entries {
        assert_eq!(entry["operation"], "not-impl", "{entry}");
        let runtime = entry["runtime"].as_str().expect("runtime citation");
        let (path, line) = runtime.rsplit_once(':').expect("runtime line number");
        let line = line.parse::<usize>().expect("numeric runtime line");
        let source = std::fs::read_to_string(workspace.join(path))
            .unwrap_or_else(|error| panic!("read {runtime}: {error}"));
        let target = source
            .lines()
            .nth(line - 1)
            .unwrap_or_else(|| panic!("runtime line out of range: {runtime}"));
        assert!(
            target.contains("notImpl(") || target.contains("notImplValue("),
            "{runtime} does not point to a not-impl refusal: {target:?}"
        );
        let module = entry["module"].as_str().expect("overlay module");
        if !modules.iter().any(|known| known == module) {
            modules.push(module.to_owned());
        }
    }

    let mut source = String::new();
    for (index, module) in modules.iter().enumerate() {
        let specifier = serde_json::to_string(&format!("#/bot/{module}")).unwrap();
        source.push_str(&format!("import * as overlay{index} from {specifier};\n"));
    }
    let module_names = (0..modules.len())
        .map(|index| format!("overlay{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    source.push_str(&format!("const overlayModules = [{module_names}];\n"));
    let cases: Vec<_> = entries
        .iter()
        .map(|entry| {
            let module = entry["module"].as_str().expect("overlay module");
            let module_index = modules
                .iter()
                .position(|known| known == module)
                .expect("registered overlay module");
            serde_json::json!([module_index, entry["selector"]])
        })
        .collect();
    source.push_str(&format!(
        "const overlayCases = {};\n",
        serde_json::to_string(&cases).unwrap()
    ));
    source.push_str(
        r#"
export default class NotImplOverlayProbe extends LoopingBot {
    async loop() {
        const unexpected = [];
        for (const [moduleIndex, selector] of overlayCases) {
            try {
                const parts = selector.split('.');
                const namespace = overlayModules[moduleIndex];
                let owner = namespace;
                if (parts.length > 1) {
                    const rootName = parts.shift();
                    owner = namespace[rootName];
                    if (owner === undefined && namespace.default?.name === rootName) {
                        owner = namespace.default;
                    }
                }
                while (parts.length > 1) owner = owner[parts.shift()];
                const memberName = parts.at(-1);
                let receiver = owner;
                let member = owner[memberName];
                if (typeof owner === 'function' && owner.prototype && member === undefined) {
                    receiver = Object.create(owner.prototype);
                    member = Reflect.get(owner.prototype, memberName, receiver);
                }
                if (typeof member === 'function') await member.call(receiver);
                else member.valueOf();
                unexpected.push(`${selector} returned`);
            } catch (error) {
                const message = String(error?.message ?? error);
                if (!message.includes('not impl:') || !message.includes(selector.split('.').at(-1))) {
                    unexpected.push(`${selector}: ${message}`);
                }
            }
        }
        globalThis.__overlayUnexpected = unexpected;
    }
}
"#,
    );

    let isolate =
        LoadIsolate::spawn(source, LoadShape::CompatClass, vec![]).expect("overlay probe loads");
    isolate.on_game_tick(1);
    let unexpected = isolate
        .probe("globalThis.__overlayUnexpected")
        .expect("overlay probe result");
    isolate.join();
    assert_eq!(
        unexpected,
        serde_json::json!([]),
        "overlay refusals: {unexpected}"
    );
}

/// Writes `src/shim/declared_surface.js` from the checked-in fixture, so the
/// generated bundle refreshes without the catalog tree.
#[test]
#[ignore]
fn regen_declared_surface_from_fixture() {
    let exports = load_fixture().expect("js_declared_abi.json");
    write_declared_surface(&exports).expect("write declared_surface.js");
}

/// The checked-in bundle is exactly what the generator renders from the
/// pinned fixture (no hand edit can drift it).
#[test]
fn declared_surface_matches_generator() {
    let exports = load_fixture().expect("js_declared_abi.json");
    let expected = render_declared_surface(&exports);
    let path = declared_surface_path();
    let actual = std::fs::read_to_string(&path).expect("read declared_surface.js");
    assert!(
        actual == expected,
        "{} is stale; run: cargo test -p script --test declared_abi \
         regen_declared_surface_from_fixture -- --ignored",
        path.display()
    );
}
