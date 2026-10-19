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
        globalThis.__gatheringMapSquare = api.MAP_SQUARE;
        globalThis.__probe = await Promise.all([
            use(() => api.ALL_FISHING_GEAR_NAMES.includes('net')),
            use(() => String(api.COINS)),
            use(() => api.FISHING_LOCATIONS.length),
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
    let map_square = iso
        .probe("__gatheringMapSquare")
        .expect("completed Gathering constant");
    iso.join();
    assert_eq!(map_square.as_i64(), Some(64));
    let rows = probe.as_array().cloned().unwrap_or_default();
    assert_eq!(
        rows.len(),
        6,
        "every remaining stub use must be probed: {probe:?}"
    );
    for (i, row) in rows.iter().enumerate() {
        let text = row.as_str().unwrap_or("");
        assert!(
            text.contains("not impl"),
            "stub use {i} must throw not impl, got {text:?}"
        );
    }
    let bank_pace = rows[5].as_str().unwrap_or("");
    assert!(
        bank_pace.contains("not impl: bankPace"),
        "unimplemented call must name bankPace, got {bank_pace:?}"
    );
}

fn compat_overlay_entries(workspace: &std::path::Path) -> Vec<serde_json::Value> {
    let root = workspace.join("crates/script/compat-js");
    [
        "overlay.json",
        "overlay-native.json",
        "overlay-not-impl.json",
        "overlay-scene.json",
    ]
    .into_iter()
    .flat_map(|name| {
        serde_json::from_str::<Vec<serde_json::Value>>(
            &std::fs::read_to_string(root.join(name)).expect("read compatibility overlay"),
        )
        .expect("parse compatibility overlay")
    })
    .collect()
}

fn assert_overlay_citations(workspace: &std::path::Path, entries: &[serde_json::Value]) {
    for entry in entries {
        let id = entry["id"].as_str().expect("overlay id");
        let runtime = entry["runtime"].as_str().expect("overlay runtime citation");
        let (path, line) = runtime.rsplit_once(':').expect("runtime line number");
        let line = line.parse::<usize>().expect("numeric runtime line");
        let source = std::fs::read_to_string(workspace.join(path))
            .unwrap_or_else(|error| panic!("read {runtime} for {id}: {error}"));
        let target = source
            .lines()
            .nth(line.checked_sub(1).expect("one-based runtime line"))
            .unwrap_or_else(|| panic!("runtime line out of range: {runtime} ({id})"));
        assert!(
            !target.trim().is_empty(),
            "blank runtime citation: {runtime} ({id})"
        );

        let selector = entry["selector"].as_str().expect("overlay selector");
        let declaration = entry["declaration"].as_str().unwrap_or_default();
        let type_only = ["export interface ", "interface ", "export type ", "type "]
            .iter()
            .any(|prefix| declaration.trim_start().starts_with(prefix));
        if !selector.starts_with('$') && !type_only {
            let symbol = selector.rsplit('.').next().expect("selector symbol");
            assert!(
                target.contains(symbol),
                "runtime citation {runtime} for {id} does not name selector {selector}: {target:?}"
            );
        }
    }
}

fn unconditional_refusal_exports(source: &str) -> Vec<(String, bool)> {
    let lines: Vec<_> = source.lines().collect();
    let mut refusals = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if let Some(declaration) = trimmed.strip_prefix("export const ") {
            if let Some((name, initializer)) = declaration.split_once('=') {
                if initializer.trim_start().starts_with("notImplValue(") {
                    refusals.push((name.trim().to_owned(), true));
                }
            }
        }

        let function = trimmed
            .strip_prefix("export async function ")
            .or_else(|| trimmed.strip_prefix("export function "));
        let Some(signature) = function else {
            continue;
        };
        let Some((name, _)) = signature.split_once('(') else {
            continue;
        };
        let mut body = String::new();
        let mut depth = 0_usize;
        let mut opened = false;
        'scan: for row in lines.iter().skip(index) {
            for character in row.chars() {
                if !opened {
                    if character == '{' {
                        opened = true;
                        depth = 1;
                    }
                    continue;
                }
                match character {
                    '{' => {
                        depth += 1;
                        body.push(character);
                    }
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break 'scan;
                        }
                        body.push(character);
                    }
                    _ => body.push(character),
                }
            }
            if opened {
                body.push('\n');
            }
        }
        let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        if opened
            && depth == 0
            && compact.starts_with("thrownotImpl(")
            && compact.ends_with(");")
            && compact.matches(';').count() == 1
        {
            refusals.push((name.trim().to_owned(), false));
        }
    }
    refusals
}

fn collect_javascript_files(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read shim directory") {
        let entry = entry.expect("read shim directory entry");
        let path = entry.path();
        if entry.file_type().expect("shim entry type").is_dir() {
            collect_javascript_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "js") {
            files.push(path);
        }
    }
}

#[test]
fn overlay_runtime_citations_are_current() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let entries = compat_overlay_entries(&workspace);
    assert_overlay_citations(&workspace, &entries);
}

#[test]
fn unconditional_refusals_have_never_overlays() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let entries = compat_overlay_entries(&workspace);
    let mut files = Vec::new();
    collect_javascript_files(&workspace.join("crates/script/src/shim"), &mut files);
    files.sort();

    for file in files {
        let source = std::fs::read_to_string(&file).expect("read shim source");
        let relative = file
            .strip_prefix(&workspace)
            .expect("shim path under workspace")
            .to_string_lossy()
            .replace('\\', "/");
        if relative.ends_with("/declared_surface.js") {
            assert!(
                entries.iter().any(|entry| {
                    entry["selector"] == "$barrel" && entry["operation"] == "runtime-values"
                }),
                "generated barrel refusals lack their group overlay"
            );
            continue;
        }
        for (symbol, is_value) in unconditional_refusal_exports(&source) {
            let matching = entries.iter().filter(|entry| {
                let Some(runtime) = entry["runtime"].as_str() else {
                    return false;
                };
                let Some((path, _)) = runtime.rsplit_once(':') else {
                    return false;
                };
                let selector = entry["selector"].as_str().unwrap_or_default();
                path == relative && selector.rsplit('.').next() == Some(symbol.as_str())
            });
            if is_value {
                let never_declaration = format!("export const {symbol}: never;");
                assert!(
                    matching.into_iter().any(|entry| {
                        entry["declaration"].as_str().is_some_and(|declaration| {
                            declaration
                                .lines()
                                .any(|line| line.trim() == never_declaration)
                        })
                    }),
                    "refused value {relative}:{symbol} is not declared as never"
                );
            } else {
                assert!(
                    matching
                        .into_iter()
                        .any(|entry| entry["operation"] == "not-impl"),
                    "refused function {relative}:{symbol} lacks a not-impl overlay"
                );
            }
        }
    }
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
        let paintFrame;
        for (const [moduleIndex, selector] of overlayCases) {
            try {
                const parts = selector.split('.');
                const namespace = overlayModules[moduleIndex];
                let owner = namespace;
                if (parts.length > 1) {
                    const rootName = parts.shift();
                    if (rootName === 'PaintFrame') {
                        paintFrame ??= namespace.Paint.begin(null, {});
                        owner = paintFrame;
                    } else {
                        owner = namespace[rootName];
                        if (owner === undefined && namespace.default?.name === rootName) {
                            owner = namespace.default;
                        }
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

#[test]
fn t2_1_abi_contract_covers_exports_families_and_visible_debt() {
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("../compat-js/abi-contract.json"))
            .expect("generated ABI contract");
    let graph: serde_json::Value =
        serde_json::from_str(include_str!("../compat-js/family-predecessors.json"))
            .expect("generated family predecessors");
    assert_eq!(contract["endpoint"], "T2.1");
    assert_eq!(contract["activation_state"], "partially-activated");
    assert_eq!(
        contract["active_families"],
        serde_json::json!([
            "bank",
            "chatbox",
            "cooking",
            "cow",
            "gathering",
            "inventory",
            "runecrafting",
            "skills",
            "world-primitives"
        ])
    );

    let family_rows = graph["families"].as_array().expect("family rows");
    let family_ids: std::collections::HashSet<&str> = family_rows
        .iter()
        .map(|row| row["family_id"].as_str().expect("family id"))
        .collect();
    let mut export_names = std::collections::HashSet::new();
    for row in contract["compatibility_name_maps"]
        .as_array()
        .expect("compatibility name maps")
    {
        let name = row["public_name"].as_str().expect("public name");
        assert!(
            export_names.insert(name),
            "duplicate compatibility export {name}"
        );
        assert!(
            family_ids.contains(row["family_id"].as_str().expect("family id")),
            "unclassified compatibility export {name}"
        );
    }
    let fixture = load_fixture().expect("declared ABI fixture");
    assert_eq!(
        export_names.len(),
        fixture.len(),
        "compatibility map/export coverage"
    );
    for export in fixture {
        assert!(
            export_names.contains(export.name.as_str()),
            "missing compatibility name map for {}",
            export.name
        );
    }

    for declaration in contract["declarations"].as_array().expect("declarations") {
        let name = declaration["name"].as_str().expect("declaration name");
        assert!(
            family_ids.contains(declaration["family_id"].as_str().expect("family id")),
            "unclassified declaration {name}"
        );
        for member in declaration["members"].as_array().expect("members") {
            let member_name = member["name"].as_str().expect("member name");
            assert!(
                family_ids.contains(member["family_id"].as_str().expect("family id")),
                "unclassified member {name}.{member_name}"
            );
            for signature in member["signatures"].as_array().expect("member overloads") {
                assert!(
                    family_ids.contains(signature["family_id"].as_str().expect("family id")),
                    "unclassified overload {name}.{member_name}"
                );
            }
        }
        for signature in declaration["signatures"].as_array().expect("overloads") {
            assert!(
                family_ids.contains(signature["family_id"].as_str().expect("family id")),
                "unclassified overload of {name}"
            );
        }
    }
    for debt in contract["not_impl_debt"].as_array().expect("not-impl debt") {
        assert_eq!(debt["state"], "not-impl");
        assert!(
            debt["member"]
                .as_str()
                .is_some_and(|member| !member.is_empty()),
            "not-impl debt must name its member: {debt}"
        );
        assert!(
            family_ids.contains(debt["family_id"].as_str().expect("family id")),
            "unclassified not-impl debt: {debt}"
        );
    }
    for callback in contract["callbacks"].as_array().expect("callback rows") {
        let member = callback["member"].as_str().expect("callback member");
        assert!(!member.is_empty(), "unnamed callback member");
        assert!(
            ["awaited", "call-sync", "ask"]
                .iter()
                .any(|path| callback["path"] == *path),
            "unknown callback path for {member}"
        );
        assert!(
            ["used", "ignored"]
                .iter()
                .any(|value| callback["value"] == *value),
            "unknown callback value use for {member}"
        );
        assert!(
            family_ids.contains(callback["family_id"].as_str().expect("family id")),
            "unclassified callback member {member}"
        );
    }
    for family in family_rows {
        let id = family["family_id"].as_str().expect("family id");
        let predecessors = family["predecessors"]
            .as_array()
            .expect("family predecessors");
        for required in ["F0", "T1.3", "T2.1"] {
            assert!(
                predecessors.iter().any(|row| row == required),
                "family {id} is missing predecessor {required}"
            );
        }
        if family["nav_consuming"]
            .as_bool()
            .expect("nav classification")
        {
            assert!(
                predecessors.iter().any(|row| row == "T5.1"),
                "nav-consuming family {id} is missing T5.1"
            );
        }
        let canonical = family["canonical_endpoint_predecessors"]
            .as_array()
            .expect("canonical endpoint predecessors");
        let canonical_names: Vec<_> = canonical
            .iter()
            .map(|row| row.as_str().expect("canonical predecessor"))
            .collect();
        let expected_canonical: Vec<_> = if id == "bank" {
            vec!["F0", "T1.3", "T2.2", "T5.1"]
        } else {
            predecessors
                .iter()
                .map(|row| row.as_str().expect("predecessor"))
                .collect()
        };
        assert_eq!(
            canonical_names, expected_canonical,
            "family {id} canonical edge"
        );
        assert!(
            !predecessors.iter().any(|row| row == "consolidated backend"),
            "family {id} has a vague predecessor"
        );
    }
}
