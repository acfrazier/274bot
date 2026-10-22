//! Cold native entity rows must be born with fast properties.
//!
//! This is a `harness = false` executable so `--allow-natives-syntax` can be
//! set before any V8 platform exists. It posts rows through the public
//! snapshot encoder and reads them back without running a JavaScript entity
//! query, i.e. without the old spread's accidental transition warming.

use script::isolate_fb::{
    encode_snapshot, encode_snapshot_delta, SceneEntityInput, SnapshotInput, TileInput,
};
use script::{LoadIsolate, LoadShape};

mod common;

const COMMON_FIELDS: [&str; 23] = [
    "index",
    "id",
    "name",
    "x",
    "z",
    "level",
    "tile",
    "distance",
    "health",
    "max_health",
    "totalHealth",
    "in_combat",
    "animating",
    "actions",
    "reachable",
    "reachable_adj",
    "combat_level",
    "target_kind",
    "target_index",
    "faceEntity",
    "size",
    "nx",
    "nz",
];

const NPC_FIELDS: [&str; 25] = [
    "index",
    "id",
    "name",
    "x",
    "z",
    "level",
    "tile",
    "distance",
    "health",
    "max_health",
    "totalHealth",
    "in_combat",
    "animating",
    "actions",
    "reachable",
    "reachable_adj",
    "combat_level",
    "target_kind",
    "target_index",
    "faceEntity",
    "size",
    "nx",
    "nz",
    "network_x",
    "network_z",
];

const BOT_SOURCE: &str = r#"
export default class RowShapeProbe extends LoopingBot {
    loop() {}
}
"#;

/// Rows posted cold, before any query: one 25-field NPC (size 2, network
/// coordinates) and one 23-field NPC (size 0), plus null names in every
/// category.
fn entity<'a>(
    index: i32,
    name: Option<&'a str>,
    x: i32,
    size: i32,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index,
        id: 100 + index,
        name,
        x,
        z: 3200 + index,
        level: 1,
        distance: 2,
        health: 7,
        max_health: 11,
        in_combat: false,
        animating: true,
        actions,
        reachable: true,
        reachable_adj: false,
        combat_level: 3,
        target_kind: 0,
        target_index: -1,
        size,
        nx: x,
        nz: 3200 + index,
        shape: 0,
        angle: 0,
    }
}

fn snapshot<'a>(
    tick: u64,
    npcs: &'a [SceneEntityInput<'a>],
    locs: &'a [SceneEntityInput<'a>],
    players: &'a [SceneEntityInput<'a>],
    ground: &'a [SceneEntityInput<'a>],
) -> SnapshotInput<'a> {
    let mut input: SnapshotInput<'a> = common::ingame_snapshot();
    input.tick = tick;
    input.here = Some(TileInput {
        x: 3201,
        z: 3201,
        level: 1,
    });
    input.npcs = npcs;
    input.locs = locs;
    input.players = players;
    input.ground = ground;
    input.self_slot = 1;
    input
}

fn probe(iso: &LoadIsolate, expr: &str) -> serde_json::Value {
    iso.probe(expr)
        .unwrap_or_else(|error| panic!("probe failed: {error}"))
}

const READ_FACTS: &str = r#"(() => {
    const fast = new Function('o', 'return %HasFastProperties(o)');
    const snapshot = globalThis.__rs2b0t_host.snapshot;
    const facts = (row) => {
        const name = Object.getOwnPropertyDescriptor(row, 'name');
        return {
            fast: fast(row),
            keys: Object.keys(row),
            prototype: Object.getPrototypeOf(row) === Object.prototype,
            name: row.name,
            x: row.x,
            tile: row.tile && [row.tile.x, row.tile.z, row.tile.level],
            actions: row.actions,
            network: Object.prototype.hasOwnProperty.call(row, 'network_x'),
            networkX: row.network_x,
            networkZ: row.network_z,
            nameWritable: name.writable,
            nameEnumerable: name.enumerable,
            nameConfigurable: name.configurable,
        };
    };
    return {
        npcs: snapshot.npcs.map(facts),
        locs: snapshot.locs.map(facts),
        players: snapshot.players.map(facts),
        ground: snapshot.ground.map(facts),
        identities: [snapshot.npcs[0] === globalThis.__saved.npc,
            snapshot.locs[0] === globalThis.__saved.loc,
            snapshot.players[0] === globalThis.__saved.player,
            snapshot.ground[0] === globalThis.__saved.ground],
    };
})()"#;

const SAVE_IDENTITIES: &str = r#"(() => {
    const snapshot = globalThis.__rs2b0t_host.snapshot;
    globalThis.__saved = {
        npc: snapshot.npcs[0],
        loc: snapshot.locs[0],
        player: snapshot.players[0],
        ground: snapshot.ground[0],
    };
    return true;
})()"#;

fn expected_keys(row: &serde_json::Value) -> &'static [&'static str] {
    if row["network"].as_bool().unwrap() {
        &NPC_FIELDS
    } else {
        &COMMON_FIELDS
    }
}

fn assert_fast_rows(facts: &serde_json::Value, label: &str) {
    for category in ["npcs", "locs", "players", "ground"] {
        let rows = facts[category]
            .as_array()
            .unwrap_or_else(|| panic!("{label}: missing {category} rows"));
        assert!(!rows.is_empty(), "{label}: {category} has no rows");
        for (index, row) in rows.iter().enumerate() {
            assert!(
                row["fast"].as_bool().unwrap_or(false),
                "{label}: {category}[{index}] must be fast: {row}"
            );
            assert!(
                row["prototype"].as_bool().unwrap_or(false),
                "{label}: {category}[{index}] must keep Object.prototype"
            );
            assert!(
                row["nameWritable"].as_bool().unwrap_or(false),
                "{label}: {category}[{index}] name must stay writable"
            );
            assert!(
                row["nameEnumerable"].as_bool().unwrap_or(false),
                "{label}: {category}[{index}] name must stay enumerable"
            );
            assert!(
                row["nameConfigurable"].as_bool().unwrap_or(false),
                "{label}: {category}[{index}] name must stay configurable"
            );
            let keys: Vec<&str> = row["keys"]
                .as_array()
                .expect("keys array")
                .iter()
                .map(|key| key.as_str().expect("string key"))
                .collect();
            assert_eq!(
                keys,
                expected_keys(row),
                "{label}: {category}[{index}] field order"
            );
        }
    }
}

fn main() {
    // Must run before V8 exists in this process: the isolated executable
    // exists precisely to keep this flag off every other test binary.
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    script::bootstrap_v8_platform().expect("process-main V8 bootstrap");
    let iso = LoadIsolate::spawn(BOT_SOURCE.into(), LoadShape::CompatClass, vec![])
        .expect("spawn row-shape isolate");
    probe(&iso, "true");

    let actions = ["Attack".to_string()];
    let cold_npcs = [
        entity(0, Some("Goblin"), 3200, 2, &actions),
        entity(1, None, 3210, 0, &actions),
    ];
    let cold_locs = [
        entity(2, Some("Tree"), 3220, 0, &actions),
        entity(3, None, 3230, 0, &actions),
    ];
    let cold_players = [entity(4, None, 3240, 0, &actions)];
    let cold_ground = [entity(5, Some("Coins"), 3250, 0, &actions)];
    let cold = snapshot(1, &cold_npcs, &cold_locs, &cold_players, &cold_ground);

    let (bytes, mut fingerprint) = encode_snapshot_delta(None, &cold, false);
    assert!(iso.post_snapshot(bytes), "cold keyframe refused");
    probe(&iso, SAVE_IDENTITIES);
    let facts = probe(&iso, READ_FACTS);
    assert_fast_rows(&facts, "cold keyframe");
    assert_eq!(facts["npcs"][0]["name"], "Goblin");
    assert_eq!(facts["npcs"][0]["networkX"], 3201);
    assert_eq!(facts["npcs"][0]["networkZ"], 3201);
    assert_eq!(facts["npcs"][0]["tile"], serde_json::json!([3200, 3200, 1]));
    assert_eq!(facts["npcs"][0]["actions"], serde_json::json!(["Attack"]));
    assert!(!facts["npcs"][1]["network"].as_bool().unwrap());
    assert!(facts["npcs"][1]["name"].is_null());
    assert!(facts["locs"][1]["name"].is_null());
    assert!(facts["players"][0]["name"].is_null());

    // Replacement through a delta: the same categories arrive with new
    // values, and every new row must still be born fast without a query.
    let replaced_npcs = [
        entity(10, None, 3300, 1, &actions),
        entity(11, Some("Rat"), 3310, 0, &actions),
    ];
    let replaced_locs = [entity(12, Some("Rock"), 3320, 0, &actions)];
    let replaced_players = [entity(13, Some("Player"), 3330, 0, &actions)];
    let replaced_ground = [entity(14, None, 3340, 0, &actions)];
    let replaced = snapshot(
        2,
        &replaced_npcs,
        &replaced_locs,
        &replaced_players,
        &replaced_ground,
    );
    let (bytes, next) = encode_snapshot_delta(Some(&fingerprint), &replaced, false);
    fingerprint = next;
    assert!(iso.post_snapshot(bytes), "replacement delta refused");
    let facts = probe(&iso, READ_FACTS);
    assert_fast_rows(&facts, "replacement");
    assert_eq!(
        facts["identities"],
        serde_json::json!([false, false, false, false])
    );
    assert_eq!(facts["npcs"][0]["networkX"], 3300);
    assert_eq!(facts["npcs"][1]["name"], "Rat");
    assert!(facts["npcs"][0]["name"].is_null());
    assert!(facts["ground"][0]["name"].is_null());
    probe(&iso, SAVE_IDENTITIES);

    // Omission: only `tick` changes, so the category vectors stay absent
    // from the delta and the previously posted rows remain the published
    // snapshot values.
    let omitted = snapshot(
        3,
        &replaced_npcs,
        &replaced_locs,
        &replaced_players,
        &replaced_ground,
    );
    let (bytes, _) = encode_snapshot_delta(Some(&fingerprint), &omitted, false);
    assert!(iso.post_snapshot(bytes), "omission delta refused");
    let facts = probe(&iso, READ_FACTS);
    assert_fast_rows(&facts, "omission");
    assert_eq!(
        facts["identities"],
        serde_json::json!([true, true, true, true])
    );

    // Session reset releases the templates with the row roots; the next
    // keyframe must recreate both on this isolate and stay fast.
    iso.reset_session_work(None);
    let recreated_npcs = [
        entity(20, Some("Guard"), 3400, 2, &actions),
        entity(21, None, 3410, 0, &actions),
    ];
    let recreated_locs = [entity(22, None, 3420, 0, &actions)];
    let recreated_players = [entity(23, Some("Player"), 3430, 0, &actions)];
    let recreated_ground = [entity(24, Some("Bones"), 3440, 0, &actions)];
    let recreated = snapshot(
        4,
        &recreated_npcs,
        &recreated_locs,
        &recreated_players,
        &recreated_ground,
    );
    assert!(
        iso.post_snapshot(encode_snapshot(&recreated)),
        "recreation keyframe refused"
    );
    let facts = probe(&iso, READ_FACTS);
    assert_fast_rows(&facts, "reset and recreation");
    assert_eq!(
        facts["identities"],
        serde_json::json!([false, false, false, false])
    );
    assert_eq!(facts["npcs"][0]["name"], "Guard");
    assert_eq!(facts["npcs"][0]["networkX"], 3401);

    let _ = iso.join();
}
