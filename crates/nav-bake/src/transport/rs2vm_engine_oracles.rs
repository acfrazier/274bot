//! Real engine handlers, objects, and zone lists are the parity authority.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use api::test_support::local_asset_path;
use client::config::Cache;
use client::io::JagFile;
use rs2vm::commands::{js_arith, ArithOp};
use rs2vm::world::{Alloc, LocInfo, PlacedId, PlacedLoc, Tri, World, WorldOverlay, ZoneKey};
use rs2vm::{Coord, LocTypeId, Ticks};
use serde::Deserialize;

use super::nav_world::NavWorld;

// A separate event loop enforces the deadline even during synchronous map loads.
const ENGINE_ORACLE_RUNNER: &str = r#"
let child;
const timer = setTimeout(() => {
    console.error('rs2vm real engine oracle exceeded its 120-second process deadline');
    child?.kill('SIGKILL');
}, 120000);
const root = process.env.RS2VM_ORACLE_ENGINE;
child = Bun.spawn({
    cmd: [process.execPath, '--tsconfig-override', root + '/tsconfig.json', root + '/tools/server/rsa.ts'],
    cwd: process.cwd(),
    stdin: 'inherit', stdout: 'inherit', stderr: 'inherit'
});
let status = await child.exited;
if (status === 0) {
    child = Bun.spawn({
        cmd: [process.execPath, '--tsconfig-override', root + '/tsconfig.json', '--eval', process.env.RS2VM_ORACLE_SOURCE],
        cwd: process.cwd(),
        stdin: 'inherit', stdout: 'inherit', stderr: 'inherit'
    });
    status = await child.exited;
}
clearTimeout(timer);
process.exit(status);
"#;

const ENGINE_ORACLE: &str = r#"
const timer = setTimeout(() => { console.error('rs2vm engine oracle exceeded 120 seconds'); process.exit(93); }, 120000);
timer.unref();
const zoneKeys = JSON.parse(await Bun.stdin.text());
const root = process.env.RS2VM_ORACLE_ENGINE;
const content = process.env.RS2VM_ORACLE_CONTENT;
const load = async path => await import(root + '/src/' + path + '.ts');
const { default: Environment } = await load('util/Environment');
Environment.build.srcDir = content;
Environment.node.members = true;
const { default: World } = await load('engine/World');
const { default: ScriptFile } = await load('engine/script/ScriptFile');
const { default: ScriptState } = await load('engine/script/ScriptState');
const { ScriptOpcode } = await load('engine/script/ScriptOpcode');
const { default: OpcodePointers } = await load('engine/script/ScriptOpcodePointers');
const readNames = [
    'PUSH_VARP', 'PUSH_VARN', 'PUSH_VARS', 'PUSH_VARBIT', 'UID', 'COORD', 'STAT',
    'INV_TOTAL', 'INV_TOTALCAT', 'INV_GETOBJ', 'MAP_MEMBERS', 'MAP_CLOCK', 'GETTIMER',
    'TEXT_GENDER', 'LOC_COORD', 'LOC_TYPE', 'LOC_ANGLE', 'LOC_SHAPE', 'LOC_CATEGORY',
    'LOC_PARAM', 'LOC_NAME', 'LC_PARAM', 'LOC_FINDALLZONE', 'LOC_FINDNEXT', 'NPC_FIND',
    'NPC_FINDALL', 'NPC_FINDALLANY', 'NPC_HUNTALL', 'NPC_FINDNEXT', 'NPC_COORD',
    'NPC_TYPE', 'NPC_CATEGORY', 'NPC_GETMODE', 'NPC_NAME', 'NC_NAME',
    'SPLIT_PAGECOUNT', 'SPLIT_LINECOUNT', 'SPLIT_GET', 'SPLIT_GETANIM'
];
const read_pointers = readNames.map(name => {
    const opcode = ScriptOpcode[name];
    if (typeof opcode !== 'number') throw new Error('missing real engine read opcode ' + name);
    const pointers = OpcodePointers[opcode];
    return [name, [...(pointers?.require ?? []), ...(pointers?.require2 ?? [])]];
});
const { default: NumberOps } = await load('engine/script/handlers/NumberOps');
const { default: LocOps } = await load('engine/script/handlers/LocOps');
const { default: Loc } = await load('engine/entity/Loc');
const { EntityLifeCycle } = await load('engine/entity/EntityLifeCycle');
const { CoordGrid } = await load('engine/CoordGrid');
const { default: LocType } = await load('cache/config/LocType');
for (const name of ['ParamType', 'LocType']) {
    const { default: type } = await load('cache/config/' + name);
    type.load(root + '/data/pack');
}
const script = new ScriptFile(0);
script.intOperands = [0];
const fresh = () => { const state = new ScriptState(script); state.pc = 0; return state; };
const arithmetic = [];
const inputs = [
    ['MULTIPLY', 2147483647, 2147483647], ['MULTIPLY', 65536, 65536],
    ['DIVIDE', 7, 0], ['DIVIDE', -7, 2], ['MODULO', -7, 3], ['MODULO', 7, 0],
    ['ADD', 2147483647, 1], ['SUB', -2147483648, 1]
];
for (const [name, a, b] of inputs) {
    const state = fresh();
    state.pushInt(a); state.pushInt(b);
    NumberOps[ScriptOpcode[name]](state);
    arithmetic.push(state.popInt());
}
const types = LocType.configs.filter(type => type && type.id < 16384 && !type.blockwalk).slice(0, 3).map(type => type.id);
if (types.length !== 3) throw new Error('three real non-blocking loc types are required');
const [a, b, c] = types;
const at = (x, z) => CoordGrid.packCoord(0, x, z);
const call = (state, name, values = [], secondary = false) => {
    script.intOperands[0] = secondary ? 1 : 0;
    for (const value of values) state.pushInt(value);
    LocOps[ScriptOpcode[name]](state);
};
const place = (x, z, type) => {
    const loc = new Loc(0, x, z, LocType.get(type).width, LocType.get(type).length, EntityLifeCycle.RESPAWN, type, 0, 0);
    World.gameMap.getZone(x, z, 0).addStaticLoc(loc);
    return loc;
};
const advance = (loc, ticks) => {
    for (let tick = 0; tick < ticks; tick++) {
        World.currentTick++;
        loc.turn();
    }
};
const bind = (state, x, z, type) => {
    call(state, 'LOC_FIND', [at(x, z), type]);
    if (state.popInt() !== 1) throw new Error('placed loc must be found');
};
const life = {};
{
    const state = fresh();
    call(state, 'LOC_ADD', [at(16300, 16300), a, 0, 0, 500]);
    const old = state.activeLoc;
    call(state, 'LOC_FIND', [at(16300, 16300), a], true);
    if (state.popInt() !== 1) throw new Error('secondary loc_find must find C1');
    call(state, 'LOC_DEL', [3]);
    call(state, 'LOC_ADD', [at(16300, 16300), b, 0, 0, 500]);
    const current = state.activeLoc;
    call(state, 'LOC_CHANGE', [c, 500], true);
    call(state, 'LOC_TYPE');
    life.old_reference = [old !== current, state.popInt() === b, old.type === a];
}
{
    const placed = place(16301, 16300, a);
    const state = fresh(); bind(state, 16301, 16300, a);
    call(state, 'LOC_DEL', [500]);
    call(state, 'LOC_ADD', [at(16301, 16300), b, 0, 0, 3]);
    const same = state.activeLoc === placed;
    advance(placed, 3);
    call(state, 'LOC_TYPE'); const reverted = state.popInt() === a;
    call(state, 'LOC_FIND', [at(16301, 16300), a]);
    life.delete_then_add = [same, reverted, state.popInt() === 1];
}
{
    const placed = place(16302, 16300, a);
    const state = fresh(); bind(state, 16302, 16300, a);
    call(state, 'LOC_DEL', [3]);
    call(state, 'LOC_DEL', [500]);
    const unchanged_life = placed.lifecycleTick === 3;
    advance(placed, 3);
    call(state, 'LOC_FIND', [at(16302, 16300), a]);
    life.second_delete = [unchanged_life, state.popInt() === 1];
}
{
    const first = place(16303, 16300, a);
    const second = place(16303, 16300, b);
    const state = fresh();
    call(state, 'LOC_ADD', [at(16303, 16300), c, 0, 0, 500]);
    const selected_first = state.activeLoc === first;
    call(state, 'LOC_ADD', [at(16303, 16300), a, 0, 0, 500]);
    const selected_second = state.activeLoc === second;
    const order = [...World.gameMap.getZone(16303, 16300, 0).getAllLocsSafe()].filter(loc => loc.x === 16303 && loc.z === 16300);
    life.list_order = [selected_first, selected_second, order[0] === first && order[1] === second];
}
// Use the engine's real ground and loc decoders. No server services start,
// and every relative runtime write stays in the isolated oracle directory.
const fs = await import('node:fs');
const { default: Packet } = await load('io/Packet');
const zipPath = root + '/data/pack/.cache/maps-server.zip';
let readMap;
let maps;
if (fs.existsSync(zipPath)) {
    const { unzipSync } = await import(Bun.resolveSync('fflate', root));
    const entries = unzipSync(fs.readFileSync(zipPath));
    maps = Object.keys(entries).filter(name => name[0] === 'm');
    readMap = name => entries[name];
} else {
    const directory = root + '/data/pack/server/maps/';
    maps = fs.readdirSync(directory).filter(name => name[0] === 'm');
    readMap = name => fs.readFileSync(directory + name);
}
if (maps.length === 0) throw new Error('real engine map cache is empty');
for (const name of maps) {
    const [mx, mz] = name.substring(1).split('_').map(Number);
    const lands = new Int8Array(4 * 64 * 64);
    World.gameMap.loadGround(lands, new Packet(readMap(name)), mx << 6, mz << 6);
    World.gameMap.loadLocations(lands, new Packet(readMap('l' + name.substring(1))), mx << 6, mz << 6);
}
const zones = zoneKeys.map(([x, z, level]) => ({
    key: [x, z, level],
    locs: [...World.gameMap.getZone(x << 3, z << 3, level).getAllLocsSafe()].map(loc => [loc.x, loc.z, loc.level, loc.type, loc.shape, loc.angle])
}));
await new Promise((resolve, reject) => {
    process.stdout.write('RS2VM_ORACLE_JSON=' + JSON.stringify({ arithmetic, types, life, zones, read_pointers }) + '\n',
        error => error ? reject(error) : resolve());
});
process.exit(0);
"#;

#[derive(Deserialize)]
struct EngineOracle {
    arithmetic: Vec<i32>,
    types: [i32; 3],
    life: HashMap<String, Vec<bool>>,
    zones: Vec<EngineZone>,
    read_pointers: Vec<(String, Vec<String>)>,
}

#[derive(Deserialize)]
struct EngineZone {
    key: [i32; 3],
    locs: Vec<[i32; 6]>,
}

#[derive(Default)]
struct OverlayFixture {
    locs: Vec<PlacedLoc>,
}

impl World for OverlayFixture {
    fn zone_locs(&self, zone: ZoneKey) -> &[PlacedLoc] {
        if self
            .locs
            .first()
            .is_some_and(|loc| ZoneKey::of(loc.tile) == zone)
        {
            &self.locs
        } else {
            &[]
        }
    }

    fn placed_loc(&self, id: PlacedId) -> Option<PlacedLoc> {
        self.locs.iter().find(|loc| loc.id == id).cloned()
    }

    fn zone_alloc(&self, _: Coord) -> Alloc {
        Alloc {
            on_members: Tri::Yes,
            on_free: Tri::Yes,
        }
    }
}

struct OracleDirectory(PathBuf);

impl OracleDirectory {
    fn new(revision: u16, content: &Path) -> Self {
        let parent =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/rs2vm-engine-oracles");
        std::fs::create_dir_all(&parent).expect("oracle directory parent inside the worktree");
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock after Unix epoch")
            .as_nanos();
        let path = parent.join(format!("{}-{revision}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).expect("new isolated oracle directory");
        let directory = Self(path);
        let config_dir = directory.0.join("data/config");
        std::fs::create_dir_all(&config_dir).expect("isolated engine config directory");
        let config = serde_json::json!({
            "engine": { "revision": revision },
            "node": { "members": true },
            "login": { "enabled": false },
            "friend": { "enabled": false },
            "logger": { "enabled": false },
            "build": { "srcDir": content }
        });
        serde_json::to_writer(
            std::fs::File::create(config_dir.join("world.json")).expect("isolated world config"),
            &config,
        )
        .expect("write isolated engine settings");
        directory
    }
}

impl Drop for OracleDirectory {
    fn drop(&mut self) {
        // Cleanup must not replace an earlier assertion failure.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn asset(revision: u16, kind: &str) -> Option<PathBuf> {
    let key = format!("RS2VM_{kind}_{revision}");
    local_asset_path(
        &key,
        "",
        &format!("revision {revision} {kind} for real engine parity"),
    )
}

fn ordered_world(revision: u16, engine: &Path, content: &Path) -> (NavWorld, Vec<ZoneKey>) {
    let cache = engine.join("data/pack/client");
    let jag_path = crate::bake::config_jag_for(revision, &cache).expect("selected config jag path");
    let jag = JagFile::new(std::fs::read(jag_path).expect("selected config jag"));
    let locs = Cache::unpack(&jag).locs;
    let world =
        NavWorld::load(&content.join("maps"), content, &locs).expect("real ordered nav world");
    let mut zones = BTreeSet::new();
    // Select from raw placements, not the adapter's active set: an incorrect
    // active filter must not hide an entire engine zone from this oracle.
    for entry in std::fs::read_dir(content.join("maps")).expect("real map directory") {
        let path = entry.expect("map directory entry").path();
        let Some((mx, mz)) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(super::index::mapsquare_coords)
        else {
            continue;
        };
        let text = std::fs::read_to_string(path).expect("real source map");
        for (loc, _) in super::index::parse_jm2_locs_ordered(&text, mx, mz) {
            zones.insert((loc.x >> 3, loc.z >> 3, loc.level));
        }
    }
    assert!(
        !zones.is_empty(),
        "real world must contain placed loc zones"
    );
    zones.insert((2047, 2047, 3));
    let zones = zones
        .into_iter()
        .map(|(x, z, level)| ZoneKey { x, z, level })
        .collect();
    (world, zones)
}

fn overlay_outcomes(types: [i32; 3]) -> HashMap<String, Vec<bool>> {
    let [a, b, c] = types.map(LocTypeId);
    let info = |ty| LocInfo {
        ty,
        shape: 0,
        angle: 0,
    };
    let mut observed = HashMap::new();
    let empty = OverlayFixture::default();
    let mut overlay = WorldOverlay::default();
    let tile = Coord::new(16300, 0, 16300);
    let old = overlay.loc_add(&empty, tile, info(a), 500).expect("C1 add");
    let secondary = overlay
        .loc_find(&empty, tile, a)
        .expect("C1 lookup")
        .expect("C1 exists");
    overlay.loc_del(&empty, old, 3).expect("C1 delete");
    let current = overlay.loc_add(&empty, tile, info(b), 500).expect("C2 add");
    overlay
        .loc_change(&empty, secondary, c, 500)
        .expect("stale C1 change");
    observed.insert(
        "old_reference".to_owned(),
        vec![
            old != current,
            overlay.read(&empty, current).expect("C2").0.ty == b,
            overlay.read(&empty, old).expect("C1").0.ty == a,
        ],
    );

    let tile = Coord::new(16301, 0, 16300);
    let placed = PlacedLoc {
        id: PlacedId(0),
        tile,
        info: info(a),
    };
    let world = OverlayFixture { locs: vec![placed] };
    let mut overlay = WorldOverlay::default();
    let original = overlay
        .loc_find(&world, tile, a)
        .expect("placed lookup")
        .expect("placed loc");
    overlay
        .loc_del(&world, original, 500)
        .expect("placed delete");
    let changed = overlay
        .loc_add(&world, tile, info(b), 3)
        .expect("placed replacement");
    overlay
        .age(&world, Ticks { min: 3, max: 3 })
        .expect("three engine ticks");
    observed.insert(
        "delete_then_add".to_owned(),
        vec![
            changed == original,
            overlay.read(&world, original).expect("reverted loc").0.ty == a,
            overlay
                .loc_find(&world, tile, a)
                .expect("base lookup")
                .is_some(),
        ],
    );

    let mut overlay = WorldOverlay::default();
    overlay.loc_del(&world, original, 3).expect("first delete");
    overlay
        .loc_del(&world, original, 500)
        .expect("inactive delete");
    overlay
        .age(&world, Ticks { min: 3, max: 3 })
        .expect("initial life expires");
    let restored = overlay
        .loc_find(&world, tile, a)
        .expect("restored lookup")
        .is_some();
    observed.insert("second_delete".to_owned(), vec![restored, restored]);

    let tile = Coord::new(16303, 0, 16300);
    let world = OverlayFixture {
        locs: vec![
            PlacedLoc {
                id: PlacedId(0),
                tile,
                info: info(a),
            },
            PlacedLoc {
                id: PlacedId(1),
                tile,
                info: info(b),
            },
        ],
    };
    let mut overlay = WorldOverlay::default();
    let first = overlay
        .loc_add(&world, tile, info(c), 500)
        .expect("first replacement");
    let second = overlay
        .loc_add(&world, tile, info(a), 500)
        .expect("second replacement");
    let order = overlay
        .zone_iter(&world, ZoneKey::of(tile))
        .expect("ordered list");
    observed.insert(
        "list_order".to_owned(),
        vec![
            first == rs2vm::world::LocObj::Placed(PlacedId(0)),
            second == rs2vm::world::LocObj::Placed(PlacedId(1)),
            order == [first, second],
        ],
    );
    observed
}

fn assert_engine_parity(revision: u16) {
    let (Some(engine), Some(content)) = (asset(revision, "ENGINE"), asset(revision, "CONTENT"))
    else {
        return;
    };
    let Some(bun) = local_asset_path("RS2VM_BUN", "", "Bun for real engine parity") else {
        return;
    };
    let (world, zones) = ordered_world(revision, &engine, &content);
    let directory = OracleDirectory::new(revision, &content);
    let keys: Vec<_> = zones
        .iter()
        .map(|zone| [zone.x, zone.z, zone.level])
        .collect();
    let mut child = Command::new(bun)
        .arg("--tsconfig-override")
        .arg(engine.join("tsconfig.json"))
        .args(["--eval", ENGINE_ORACLE_RUNNER])
        .current_dir(&directory.0)
        .env("RS2VM_ORACLE_ENGINE", &engine)
        .env("RS2VM_ORACLE_SOURCE", ENGINE_ORACLE)
        .env("RS2VM_ORACLE_CONTENT", &content)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start Bun real engine oracle");
    let mut stdin = child.stdin.take().expect("oracle stdin pipe");
    serde_json::to_writer(&mut stdin, &keys).expect("write ordered world zone keys");
    drop(stdin);
    let output = child
        .wait_with_output()
        .expect("wait for real engine oracle");
    assert!(
        output.status.success(),
        "revision {revision} real engine oracle failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("engine oracle stdout UTF-8");
    let json = stdout
        .lines()
        .find_map(|line| line.strip_prefix("RS2VM_ORACLE_JSON="))
        .expect("real engine JSON result");
    let observed: EngineOracle = serde_json::from_str(json).expect("real engine result schema");
    let arithmetic = [
        (ArithOp::Multiply, i32::MAX, i32::MAX),
        (ArithOp::Multiply, 65536, 65536),
        (ArithOp::Divide, 7, 0),
        (ArithOp::Divide, -7, 2),
        (ArithOp::Modulo, -7, 3),
        (ArithOp::Modulo, 7, 0),
        (ArithOp::Add, i32::MAX, 1),
        (ArithOp::Sub, i32::MIN, 1),
    ];
    let expected: Vec<_> = arithmetic
        .into_iter()
        .map(|(op, a, b)| js_arith(op, a, b))
        .collect();
    assert_eq!(
        observed.arithmetic, expected,
        "revision {revision} engine arithmetic"
    );
    assert_eq!(
        observed.read_pointers.len(),
        39,
        "every slice-1 read row checks real engine pointers"
    );
    for (name, required) in &observed.read_pointers {
        assert!(
            !required
                .iter()
                .any(|pointer| pointer.starts_with("p_active_player")),
            "revision {revision} read row {name} requires protected access: {required:?}"
        );
    }
    assert!(
        observed.life.values().flatten().all(|&value| value),
        "revision {revision} engine life-cycle preconditions: {:?}",
        observed.life
    );
    assert_eq!(
        observed.life,
        overlay_outcomes(observed.types),
        "revision {revision} real World/Zone/Loc parity"
    );
    assert_eq!(
        observed.zones.len(),
        zones.len(),
        "every active world zone is compared"
    );
    for (actual, zone) in observed.zones.iter().zip(zones) {
        assert_eq!(actual.key, [zone.x, zone.z, zone.level]);
        let expected: Vec<_> = world
            .zone_locs(zone)
            .iter()
            .map(|loc| {
                [
                    loc.tile.x(),
                    loc.tile.z(),
                    loc.tile.level(),
                    loc.info.ty.0,
                    i32::from(loc.info.shape),
                    i32::from(loc.info.angle),
                ]
            })
            .collect();
        assert_eq!(
            actual.locs, expected,
            "revision {revision} engine getAllLocsSafe order at {zone:?}"
        );
    }
}

#[test]
fn revision_274_real_engine_arithmetic_life_cycle_and_zone_order() {
    assert_engine_parity(274);
}

#[test]
fn revision_289_real_engine_arithmetic_life_cycle_and_zone_order() {
    assert_engine_parity(289);
}
