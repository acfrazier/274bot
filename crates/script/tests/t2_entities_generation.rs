#![cfg(feature = "load")]

use script::isolate_fb::{IsolateBuf, SceneEntityInput, Snapshot, SnapshotInput};
use script::{LoadIsolate, LoadShape};

mod common;

fn entity<'a>(
    index: i32,
    id: i32,
    name: &'a str,
    (x, z, level): (i32, i32, i32),
    distance: i32,
    actions: &'a [String],
    target_index: Option<i32>,
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index,
        id,
        name: Some(name),
        x,
        z,
        level,
        distance,
        health: 7,
        max_health: 11,
        in_combat: false,
        animating: false,
        actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: if target_index.is_some() { 2 } else { 0 },
        target_index: target_index.unwrap_or(-1),
        size: 1,
        nx: x,
        nz: z,
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
    self_slot: i32,
    run_energy: i32,
    main_modal_id: i32,
) -> SnapshotInput<'a> {
    let mut input: SnapshotInput<'a> = common::ingame_snapshot();
    input.tick = tick;
    input.npcs = npcs;
    input.locs = locs;
    input.players = players;
    input.ground = ground;
    input.self_slot = self_slot;
    input.run_energy = run_energy;
    input.main_modal_id = main_modal_id;
    input
}

fn post(iso: &LoadIsolate, tick: u64, bytes: Vec<u8>) {
    assert!(iso.post_snapshot(bytes));
    iso.on_game_tick(tick);
}

const ENTITY_PAGE_API: &str = r#"
import { Npcs } from '../../api/npcs/Npcs.js';
import { Locs } from '../../api/locs/Locs.js';
import { Players } from '../../api/players/Players.js';
import { GroundItems } from '../../api/grounditems/GroundItems.js';
import EntityQuery from '../../api/query/Query.js';

const area = { minX: 3220, maxX: 3230, minZ: 3220, maxZ: 3230 };
const hostSnapshot = () => globalThis.__rs2b0t_host.snapshot;
const sameRows = (entities, rows) => entities.length === rows.length
    && entities.every((entity, index) => entity.snap === rows[index]);
const identityLabel = (entity, rows) => entity.snap === rows[0] ? 'first'
    : entity.snap === rows[1] ? 'second' : 'other';
let base;
let savedModal;
let failureCalls = 0;

function rememberBase() {
    const snapshot = hostSnapshot();
    const pages = {
        npcs: snapshot.npcs.slice(),
        locs: snapshot.locs.slice(),
        players: snapshot.players.slice(),
        ground: snapshot.ground.slice(),
    };
    base = {
        pages,
        queries: {
            npcs: Npcs.query().results(),
            locs: Locs.query().results(),
            players: Players.query().results(),
            ground: GroundItems.query().results(),
        },
        all: { npcs: Npcs.all(), players: Players.all() },
    };
    return {
        queryIdentity: [
            sameRows(base.queries.npcs, pages.npcs),
            sameRows(base.queries.locs, pages.locs),
            sameRows(base.queries.players, pages.players),
            sameRows(base.queries.ground, pages.ground),
        ],
        allIdentity: [
            sameRows(base.all.npcs, pages.npcs),
            sameRows(base.all.players, pages.players),
        ],
    };
}

function reverseAndMutatePublicRows() {
    const snapshot = hostSnapshot();
    snapshot.npcs.reverse();
    snapshot.locs.reverse();
    snapshot.players.reverse();
    snapshot.ground.reverse();
    for (const rows of Object.values(base.pages)) {
        for (const row of rows) {
            Object.assign(row, {
                name: 'edited',
                id: -900,
                index: -900,
                distance: 900,
                x: 9000,
                actions: [],
                target_kind: 0,
                target_index: 999,
            });
        }
    }
    snapshot.self_slot = 999;
    return true;
}

function replacePublicPages() {
    const snapshot = hostSnapshot();
    const poison = {
        index: 99,
        id: 999,
        name: 'poison',
        x: 9000,
        z: 9000,
        level: 0,
        distance: 0,
        actions: ['Poison'],
        target_kind: 0,
        target_index: -1,
    };
    for (const key of ['npcs', 'locs', 'players', 'ground']) {
        snapshot[key].splice(0, snapshot[key].length);
        snapshot[key] = [poison];
    }
    return true;
}

function nativeSummary() {
    const rows = {
        npcs: Npcs.query().results(),
        locs: Locs.query().results(),
        players: Players.query().results(),
        ground: GroundItems.query().results(),
    };
    const selected = [
        Npcs.query().name('Goblin').action('Attack').within(2).inside(area).results(),
        Locs.query().name('Door').action('Open').within(2).inside(area).results(),
        Players.query().name('Alice').action('Trade').within(2).inside(area).results(),
        GroundItems.query().name('Bones').action('Take').within(2).inside(area).results(),
    ];
    const predicateOrder = [];
    const predicateResult = Npcs.query().where((npc) => {
        predicateOrder.push(identityLabel(npc, base.pages.npcs));
        return npc.snap === base.pages.npcs[1];
    }).results();
    const shortCircuitOrder = [];
    let visits = 0;
    Npcs.query().where((npc) => {
        shortCircuitOrder.push(identityLabel(npc, base.pages.npcs));
        return true;
    }).forEachMatch(() => {
        visits++;
        return true;
    });

    const callerRow = {
        id: 1,
        name: 'before',
        x: 1,
        z: 1,
        level: 0,
        distance: 1,
        actions: ['Before'],
    };
    const callerQuery = EntityQuery.fromSnapshots(() => [callerRow], (row) => row);
    Object.assign(callerRow, {
        id: 88,
        name: 'caller-mutated',
        x: 9001,
        z: 9002,
        distance: 3,
        actions: ['Caller-action'],
    });
    const callerResults = callerQuery.name('caller-mutated').action('Caller-action')
        .within(3).inside({ minX: 9001, maxX: 9001, minZ: 9002, maxZ: 9002 }).results();

    return {
        ids: [
            rows.npcs.map((npc) => npc.id),
            rows.locs.map((loc) => loc.id),
            rows.players.map((player) => player.index),
            rows.ground.map((item) => item.id),
        ],
        names: [
            rows.npcs.map((npc) => npc.name),
            rows.locs.map((loc) => loc.name),
            rows.players.map((player) => player.name),
            rows.ground.map((item) => item.name),
        ],
        identity: [
            sameRows(rows.npcs, base.pages.npcs),
            sameRows(rows.locs, base.pages.locs),
            sameRows(rows.players, base.pages.players),
            sameRows(rows.ground, base.pages.ground),
        ],
        filteredIds: [
            selected[0].map((npc) => npc.id),
            selected[1].map((loc) => loc.id),
            selected[2].map((player) => player.index),
            selected[3].map((item) => item.id),
        ],
        nearest: [
            Npcs.query().nearest().id,
            Locs.query().nearest().id,
            Players.query().nearest().index,
            GroundItems.query().nearest().id,
            Npcs.nearest(1)[0].id,
        ],
        all: [
            Npcs.all().map((npc) => [npc.id, npc.name]),
            Players.all().map((player) => [player.index, player.name]),
            sameRows(Npcs.all(), base.pages.npcs),
            sameRows(Players.all(), base.pages.players),
        ],
        validityAndTargets: [
            base.all.npcs[0].valid(),
            base.all.npcs[0].targetsMe(),
            base.all.npcs[1].targetsAnotherPlayer(),
            base.all.players[0].targetsMe(),
        ],
        where: [predicateOrder, predicateResult.map((npc) => npc.id)],
        shortCircuit: [shortCircuitOrder, visits],
        callerRows: [
            callerResults.length,
            callerResults[0] === callerRow,
            callerResults[0]?.id,
            callerQuery.name('before').count(),
        ],
    };
}

function installFailure() {
    const snapshot = hostSnapshot();
    savedModal = snapshot.main_modal_id;
    Object.defineProperty(snapshot, 'main_modal_id', {
        configurable: true,
        enumerable: true,
        get() { return savedModal; },
        set(_value) {
            failureCalls++;
            throw new Error('intentional snapshot materialization failure');
        },
    });
    return savedModal;
}

function failureSummary() {
    const summary = nativeSummary();
    const snapshot = hostSnapshot();
    return {
        setterCalls: failureCalls,
        publishedNpcId: snapshot.npcs[0]?.id,
        publicSelfSlot: snapshot.self_slot,
        publicMainModal: snapshot.main_modal_id,
        summary,
    };
}

function restoreFailure() {
    Object.defineProperty(hostSnapshot(), 'main_modal_id', {
        configurable: true,
        enumerable: true,
        writable: true,
        value: savedModal,
    });
    return true;
}

function advancedSummary() {
    const snapshot = hostSnapshot();
    const npcs = Npcs.query().results();
    const locs = Locs.query().results();
    const players = Players.query().results();
    const ground = GroundItems.query().results();
    return {
        npcFacts: npcs.map((npc) => [npc.id, npc.name]),
        npcIdentity: npcs.length === snapshot.npcs.length
            && npcs.every((npc, index) => npc.snap === snapshot.npcs[index]),
        npcTargetsMe: npcs[0].targetsMe(),
        npcValid: npcs[0].valid(),
        oldFacts: [
            locs.map((loc) => loc.id),
            players.map((player) => player.index),
            ground.map((item) => item.id),
        ],
        oldIdentity: [
            sameRows(locs, base.pages.locs),
            sameRows(players, base.pages.players),
            sameRows(ground, base.pages.ground),
        ],
    };
}

globalThis.__entityPageTest = {
    rememberBase,
    reverseAndMutatePublicRows,
    replacePublicPages,
    nativeSummary,
    installFailure,
    failureSummary,
    restoreFailure,
    advancedSummary,
};

export default class EntityPageGenerationBot extends LoopingBot {
    loop() {}
}
"#;

#[test]
fn production_entity_queries_follow_only_the_successful_native_generation() {
    let attack = ["Attack".to_string()];
    let open = ["Open".to_string()];
    let trade = ["Trade".to_string()];
    let take = ["Take".to_string()];
    let base_npcs = [
        entity(2, 101, "Goblin", (3222, 3222, 0), 2, &attack, Some(0)),
        entity(3, 102, "Goblin", (3223, 3222, 0), 1, &attack, Some(8)),
    ];
    let base_locs = [
        entity(1, 201, "Door", (3222, 3223, 0), 3, &open, None),
        entity(2, 202, "Door", (3223, 3223, 0), 1, &open, None),
    ];
    let base_players = [
        entity(4, 301, "Alice", (3222, 3224, 0), 2, &trade, Some(0)),
        entity(8, 302, "Bob", (3223, 3224, 0), 1, &trade, Some(8)),
    ];
    let base_ground = [
        entity(0, 401, "Bones", (3222, 3225, 0), 2, &take, None),
        entity(1, 402, "Bones", (3223, 3225, 0), 1, &take, None),
    ];
    let base = snapshot(
        1,
        &base_npcs,
        &base_locs,
        &base_players,
        &base_ground,
        0,
        0,
        -1,
    );

    let iso = LoadIsolate::spawn(ENTITY_PAGE_API.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    assert_eq!(iso.probe("1").unwrap(), serde_json::json!(1));
    let mut encoder = IsolateBuf::new();
    let (base_bytes, base_fingerprint) = encoder.encode_snapshot_delta(None, &base, false);
    post(&iso, base.tick, base_bytes);

    assert_eq!(
        iso.probe("globalThis.__entityPageTest.rememberBase()").unwrap(),
        serde_json::json!({
            "queryIdentity": [true, true, true, true],
            "allIdentity": [true, true],
        })
    );

    assert_eq!(
        iso.probe("globalThis.__entityPageTest.reverseAndMutatePublicRows()").unwrap(),
        serde_json::json!(true)
    );
    assert_eq!(
        iso.probe("globalThis.__entityPageTest.nativeSummary()").unwrap(),
        serde_json::json!({
            "ids": [[101, 102], [201, 202], [4, 8], [401, 402]],
            "names": [
                ["Goblin", "Goblin"], ["Door", "Door"], ["Alice", "Bob"], ["Bones", "Bones"]
            ],
            "identity": [true, true, true, true],
            "filteredIds": [[101, 102], [202], [4, 8], [401, 402]],
            "nearest": [102, 202, 8, 402, 102],
            "all": [
                [[101, "Goblin"], [102, "Goblin"]],
                [[4, "Alice"], [8, "Bob"]],
                true,
                true,
            ],
            "validityAndTargets": [true, true, true, true],
            "where": [["first", "second"], [102]],
            "shortCircuit": [["first"], 1],
            "callerRows": [1, true, 88, 0],
        })
    );

    assert_eq!(
        iso.probe("globalThis.__entityPageTest.replacePublicPages()").unwrap(),
        serde_json::json!(true)
    );
    let replaced = iso.probe("globalThis.__entityPageTest.nativeSummary()").unwrap();
    assert_eq!(replaced["ids"], serde_json::json!([[101, 102], [201, 202], [4, 8], [401, 402]]));
    assert_eq!(replaced["identity"], serde_json::json!([true, true, true, true]));
    assert_eq!(replaced["all"], serde_json::json!([
        [[101, "Goblin"], [102, "Goblin"]],
        [[4, "Alice"], [8, "Bob"]],
        true,
        true,
    ]));
    assert_eq!(replaced["validityAndTargets"], serde_json::json!([true, true, true, true]));

    let failed_npcs = [entity(
        20,
        501,
        "FailedGoblin",
        (3224, 3222, 0),
        7,
        &attack,
        Some(9),
    )];
    let failed_locs = [entity(
        21,
        601,
        "FailedDoor",
        (3224, 3223, 0),
        7,
        &open,
        None,
    )];
    let failed_players = [entity(
        29,
        701,
        "FailedAlice",
        (3224, 3224, 0),
        7,
        &trade,
        Some(9),
    )];
    let failed_ground = [entity(
        22,
        801,
        "FailedBones",
        (3224, 3225, 0),
        7,
        &take,
        None,
    )];
    let failed = snapshot(
        2,
        &failed_npcs,
        &failed_locs,
        &failed_players,
        &failed_ground,
        9,
        0,
        77,
    );
    assert_eq!(
        iso.probe("globalThis.__entityPageTest.installFailure()").unwrap(),
        serde_json::json!(-1)
    );
    let (failed_bytes, failed_fingerprint) =
        encoder.encode_snapshot_delta(Some(&base_fingerprint), &failed, false);
    {
        let delta = Snapshot::from_bytes(&failed_bytes).unwrap();
        assert!(delta.has_npcs() && delta.has_locs() && delta.has_players() && delta.has_ground());
        assert!(delta.has_self_slot() && delta.has_main_modal_id());
    }
    post(&iso, failed.tick, failed_bytes);

    let after_failure = iso.probe("globalThis.__entityPageTest.failureSummary()").unwrap();
    assert_eq!(after_failure["setterCalls"], serde_json::json!(1));
    assert_eq!(after_failure["publishedNpcId"], serde_json::json!(501));
    assert_eq!(after_failure["publicSelfSlot"], serde_json::json!(9));
    assert_eq!(after_failure["publicMainModal"], serde_json::json!(-1));
    assert_eq!(
        after_failure["summary"]["ids"],
        serde_json::json!([[101, 102], [201, 202], [4, 8], [401, 402]])
    );
    assert_eq!(
        after_failure["summary"]["identity"],
        serde_json::json!([true, true, true, true])
    );
    assert_eq!(
        after_failure["summary"]["validityAndTargets"],
        serde_json::json!([true, true, true, true])
    );

    assert_eq!(
        iso.probe("globalThis.__entityPageTest.restoreFailure()").unwrap(),
        serde_json::json!(true)
    );
    let omitted = snapshot(
        3,
        &failed_npcs,
        &failed_locs,
        &failed_players,
        &failed_ground,
        9,
        1,
        77,
    );
    let (omitted_bytes, omitted_fingerprint) =
        encoder.encode_snapshot_delta(Some(&failed_fingerprint), &omitted, false);
    {
        let delta = Snapshot::from_bytes(&omitted_bytes).unwrap();
        assert!(!delta.has_npcs() && !delta.has_locs() && !delta.has_players() && !delta.has_ground());
        assert!(!delta.has_self_slot() && !delta.has_main_modal_id());
        assert!(delta.has_run_energy());
    }
    post(&iso, omitted.tick, omitted_bytes);
    let after_omission = iso.probe("globalThis.__entityPageTest.nativeSummary()").unwrap();
    assert_eq!(after_omission["ids"], serde_json::json!([[101, 102], [201, 202], [4, 8], [401, 402]]));
    assert_eq!(after_omission["identity"], serde_json::json!([true, true, true, true]));
    assert_eq!(after_omission["validityAndTargets"], serde_json::json!([true, true, true, true]));

    let fresh_npcs = [entity(
        30,
        901,
        "FreshGoblin",
        (3225, 3222, 0),
        1,
        &attack,
        Some(8),
    )];
    let advanced = snapshot(
        4,
        &fresh_npcs,
        &failed_locs,
        &failed_players,
        &failed_ground,
        8,
        1,
        77,
    );
    let (advanced_bytes, _) =
        encoder.encode_snapshot_delta(Some(&omitted_fingerprint), &advanced, false);
    {
        let delta = Snapshot::from_bytes(&advanced_bytes).unwrap();
        assert!(delta.has_npcs() && delta.has_self_slot());
        assert!(!delta.has_locs() && !delta.has_players() && !delta.has_ground());
    }
    post(&iso, advanced.tick, advanced_bytes);
    assert_eq!(
        iso.probe("globalThis.__entityPageTest.advancedSummary()").unwrap(),
        serde_json::json!({
            "npcFacts": [[901, "FreshGoblin"]],
            "npcIdentity": true,
            "npcTargetsMe": true,
            "npcValid": true,
            "oldFacts": [[201, 202], [4, 8], [401, 402]],
            "oldIdentity": [true, true, true],
        })
    );

    iso.join();
}
