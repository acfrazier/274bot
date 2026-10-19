#![cfg(feature = "load")]

use script::isolate_fb::{
    encode_snapshot, ReachViewInput, SceneEntityInput, SnapshotInput, TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

fn entity<'a>(
    index: i32,
    id: i32,
    name: Option<&'a str>,
    x: i32,
    z: i32,
    level: i32,
    distance: i32,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index,
        id,
        name,
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
        target_kind: 0,
        target_index: -1,
        size: 0,
        nx: 0,
        nz: 0,
        shape: 0,
        angle: 0,
    }
}

fn empty_snapshot<'a>(tick: u64) -> SnapshotInput<'a> {
    SnapshotInput {
        tick,
        here: Some(TileInput {
            x: 3201,
            z: 3201,
            level: 0,
        }),
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats: &[],
        booths: &[],
        nearest_booth: None,
        banks: &[],
        bank: &[],
        bank_side: &[],
        bank_open: false,
        bank_loaded: false,
        bank_generation: 0,
        count_dialog_open: false,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
        hold: false,
        ours: false,
        npcs: &[],
        locs: &[],
        players: &[],
        ground: &[],
        equipment: &[],
        chat_open: false,
        chat_continue: false,
        chat_text: None,
        chat_options: &[],
        side_tab: 0,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: Some("tester"),
        in_combat: false,
        animating: false,
        main_modal_id: -1,
        chat_modal_id: -1,
        make_products: &[],
        side_tab_ifaces: &[],
        spell_buttons: &[],
        chat_lines: &[],
        bank_note_on: -1,
        bank_note_off: -1,
        scene_state: 2,
        weight: 0,
        combat_level: 0,
        camera_yaw: 0,
        camera_pitch: 0,
        teleports_enabled: false,
        self_slot: 2,
        trade_offer_open: false,
        trade_confirm_open: false,
        trade_partner: None,
        trade_mine: &[],
        trade_theirs: &[],
        trade_side: &[],
        trade_accept_id: -1,
        trade_decline_id: -1,
        shop_open: false,
        shop_stock: &[],
        reach: ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
        user_move_intent_seq: 0,
        walk_outcome_cancel_reason: Default::default(),
    }
}

fn tick(iso: &LoadIsolate, snapshot: &SnapshotInput<'_>) -> serde_json::Value {
    iso.post_snapshot(encode_snapshot(snapshot));
    iso.on_game_tick(snapshot.tick);
    iso.probe("__probe").unwrap()
}

const ENTITY_API: &str = r#"
import { Npcs, talkOp } from '../../api/npcs/Npcs.js';
import { Locs } from '../../api/locs/Locs.js';
import { Players } from '../../api/players/Players.js';
import { GroundItems, GroundItem } from '../../api/grounditems/GroundItems.js';
import EntityQuery, { matchesEntityName } from '../../api/query/Query.js';
import { Npc } from '../../api/npcs/Npcs.js';
import { Loc } from '../../api/locs/Locs.js';
import { Player } from '../../api/players/Players.js';
import { Npc as ModelNpc } from '../../api/model/Npc.js';
import { Loc as ModelLoc } from '../../api/model/Loc.js';
import { Player as ModelPlayer } from '../../api/model/Player.js';

export default class EntityFamilyBot extends LoopingBot {
    loop() {
        const npcs = Npcs.all();
        const npc = npcs[0];
        const loc = Locs.query().first();
        const players = Players.all();
        const player = players[0];
        const item = GroundItems.query().first();
        const stale = new Npc({ index: npc.index, id: npc.id, name: 'changed' });
        const partial = new Npc({ actions: [null, 'hidden'], distance: 'unknown' });
        const partialLoc = new Loc({ actions: [null, 'hidden'], distance: 'unknown' });
        const partialPlayer = new Player({ actions: [null, 'hidden'], combat_level: 'unknown' });
        const partialItem = new GroundItem({ actions: [null, 'hidden'], count: 'unknown', distance: 'unknown' });
        const queued = [
            npc.interact('Attack'),
            loc.interact('Open'),
            player.interact('Trade'),
            item.interact('Take'),
        ];
        const chains = Npcs.query()
            .name(' goblin ', 'GOBLIN')
            .action('attack')
            .within(2)
            .inside({ minX: 3200, maxX: 3202, minZ: 3200, maxZ: 3202 })
            .results();
        const nearest = Npcs.query().nearest();
        globalThis.__probe = {
            aliases: [Npc === ModelNpc, Loc === ModelLoc, Player === ModelPlayer],
            roots: [npcs.length, players.length, chains.length],
            wrapped: [npc instanceof Npc, loc instanceof Loc, player instanceof Player],
            npc: [npc.name, npc.id, npc.index, npc.inCombat, npc.health, npc.level, npc.size,
                npc.tile().x, npc.tile().z, npc.distance(), npc.actions(),
                npc.networkOrigin().x, npc.networkOrigin().z, npc.networkTile().x,
                npc.targetsMe(), npc.targetsAnotherPlayer(), npc.valid(), stale.valid()],
            targetVariants: [npcs[1].targetsMe(), npcs[1].targetsAnotherPlayer()],
            loc: [loc.name, loc.id, loc.tile().x, loc.tile().z, loc.distance(), loc.actions()],
            player: [player.name, player.index, player.inCombat, player.combatLevel,
                player.tile().x, player.tile().z, player.distance(), player.actions(), player.targetsMe(),
                Players.query().first() instanceof Player],
            ground: [item.name, item.id, item.count, item.tile().x, item.tile().z,
                item.distance(), item.actions()],
            partial: [partial.name, partial.id ?? null, partial.health ?? null,
                partial.actions(), partial.distance()],
            partialOther: [partialLoc.name, partialLoc.id ?? null, partialLoc.distance(), partialLoc.actions(),
                partialPlayer.name, partialPlayer.index ?? null, partialPlayer.combatLevel, partialPlayer.actions(),
                partialItem.name, partialItem.id ?? null, partialItem.count, partialItem.distance(), partialItem.actions()],
            support: [talkOp(['nothing', 'Talk-to', 'hidden']),
                matchesEntityName(' Goblin ', 'goblin'), matchesEntityName(null, 'goblin'),
                matchesEntityName(7, 7)],
            queueResults: queued,
            nearest: [nearest.id, Npcs.nearest(1)[0].id],
            firstIdentity: chains[0] instanceof Npc,
            queryMethods: [Npcs.query().count(), Npcs.query().exists(), Npcs.query().first() instanceof Npc],
        };
    }
}
"#;

const QUERY_API: &str = r#"
import EntityQuery, { matchesEntityName } from '../../api/query/Query.js';

export default class QueryBot extends LoopingBot {
    loop() {
        const rows = [
            { id: 1, name: ' Goblin ', x: 0, z: 0, level: 0, distance: 5,
                actions: [null, 'hidden', 'Attack'] },
            { id: 2, name: 'goblin', x: 1, z: 0, level: 1, distance: 1,
                actions: ['Attack'] },
            { id: 3, name: 'Goblin', x: 3, z: 2, level: 0, distance: 1,
                actions: ['Attack'] },
            { id: 4, name: null, x: 0, z: 0, level: 0, distance: 0,
                actions: ['Attack'] },
            { id: 5, x: 0, z: 0, level: 0, distance: 0, actions: ['Attack'] },
        ];
        let supplies = 0;
        const wrapped = [];
        const query = EntityQuery.fromSnapshots(
            () => { supplies++; return rows; },
            function (row) {
                const entity = { ...row, raw: row, distance() { return row.distance; } };
                wrapped.push(entity);
                return entity;
            },
        );
        const chain = query.name(' goblin ', 'GOBLIN').action('ATTACK').within(5)
            .inside({ minX: 0, maxX: 3, minZ: 0, maxZ: 2 })
            .withinOf({ x: 0, z: 0, level: 99 }, 3)
            .where((entity) => entity.id !== 99);
        const results = chain.results();
        const first = chain.first();
        const nearest = chain.nearest();
        const empty = EntityQuery.fromSnapshots(() => [], (row) => row);
        const invalid = EntityQuery.fromSnapshots(
            () => [null, false, 0, '', { name: 'partial' }],
            (row) => row,
        ).results();
        const actionPositions = [
            globalThis.__rs2b0t_entities_v8(2 /* opIndex */, [null, 'hidden', 'Attack'], 'attack'),
            globalThis.__rs2b0t_entities_v8(1 /* presentOps */, [null, false, 'hidden', 'Attack']),
        ];

        const order = [];
        const raw = {
            get name() { order.push('get-name'); return 'Goblin'; },
            get x() { order.push('get-x'); return 1; },
            get z() { order.push('get-z'); return 2; },
            get level() { order.push('get-level'); return 3; },
            get actions() { order.push('get-actions'); return ['Attack']; },
            get distance() { order.push('get-distance'); return 1; },
        };
        let callbackQuery;
        callbackQuery = EntityQuery.fromSnapshots(
            function () { order.push(this === callbackQuery ? 'supply-this' : 'bad-supply-this'); return [raw]; },
            function (row) { order.push(this === callbackQuery ? 'wrap-this' : 'bad-wrap-this'); return row; },
        );
        callbackQuery.name('Goblin').action('Attack').within(1).where(function () {
            order.push(this === undefined ? 'filter-undefined-this' : 'bad-filter-this');
            return true;
        });
        callbackQuery.snapFilters.unshift(function () {
            order.push(this === undefined ? 'extra-snap-filter' : 'bad-snap-this');
            return true;
        });
        callbackQuery.entityFilters.push(function () {
            order.push(this === undefined ? 'extra-entity-filter' : 'bad-entity-this');
            return true;
        });
        const callbackResult = callbackQuery.forEachMatch(function () {
            order.push(this === undefined ? 'seen-undefined-this' : 'bad-seen-this');
            return false;
        }) === undefined;

        let iteratorNext = 0;
        let iteratorReturn = 0;
        const earlyEntity = { id: 42 };
        const early = EntityQuery.fromSnapshots(
            () => ({
                [Symbol.iterator]() {
                    return {
                        next() {
                            iteratorNext++;
                            return iteratorNext === 1
                                ? { value: { name: 'early', x: 0, z: 0, distance: 0 }, done: false }
                                : { value: undefined, done: true };
                        },
                        return() { iteratorReturn++; return { done: true }; },
                    };
                },
            }),
            () => earlyEntity,
        );
        const stopped = early.first() === earlyEntity;

        const error = new Error('same thrown object');
        const throwsSame = (run) => {
            try { run(); return false; }
            catch (caught) { return caught === error; }
        };
        const filterError = EntityQuery.fromSnapshots(() => [rows[0]], (row) => row)
            .where(() => { throw error; });
        const supplierError = EntityQuery.fromSnapshots(() => { throw error; }, (row) => row);
        const getterError = EntityQuery.fromSnapshots(
            () => [{ get name() { throw error; } }],
            (row) => row,
        );
        const wrapperError = EntityQuery.fromSnapshots(() => [rows[0]], () => { throw error; });
        const callbackErrors = [
            throwsSame(() => filterError.results()),
            throwsSame(() => supplierError.results()),
            throwsSame(() => getterError.results()),
            throwsSame(() => wrapperError.results()),
            throwsSame(() => EntityQuery.fromSnapshots(() => [rows[0]], (row) => row)
                .forEachMatch(() => { throw error; })),
        ];

        const overriddenEntity = { id: 88, distance() { return 1; } };
        let resultCalls = 0;
        const overridden = EntityQuery.fromSnapshots(() => [], (row) => row);
        overridden.results = function () {
            resultCalls++;
            if (this !== overridden) throw error;
            return [overriddenEntity];
        };
        const preferred = overridden.nearestPreferLocal(2) === overriddenEntity;

        const radiusRows = [
            { id: 'far', distance() { return 10; } },
            { id: 'near', distance() { return 1; } },
            { id: 'tie', distance() { return 1; } },
        ];
        const radiusQuery = EntityQuery.fromSnapshots(() => [], (row) => row);
        radiusQuery.results = function () { return radiusRows; };
        const radiusResults = [0, -2, 1.9, NaN, Infinity].map((radius) => {
            const result = radiusQuery.nearestPreferLocal(radius);
            return result === null ? null : result.id;
        });

        const mutableCalls = [];
        const mutableRows = ['far', 'near', 'tie'].map((id) => {
            let calls = 0;
            return {
                id,
                distance() {
                    calls++;
                    mutableCalls.push(`${id}:${calls}`);
                    if (id === 'far') return 10;
                    if (id === 'near') return calls === 1 ? 1 : 4;
                    return calls === 1 ? 2 : 0;
                },
            };
        });
        const mutableQuery = EntityQuery.fromSnapshots(() => [], (row) => row);
        mutableQuery.results = function () { return mutableRows; };
        const mutableWinner = mutableQuery.nearestPreferLocal(2);

        const customIter = EntityQuery.fromSnapshots(() => [rows[0]], (row) => row);
        let overriddenForEachThis = false;
        customIter.forEachMatch = function (visit) {
            overriddenForEachThis = this === customIter;
            visit(earlyEntity);
        };
        const overrideResults = customIter.results();

        globalThis.__probe = {
            chain: [results.map((entity) => entity.id), first.id, nearest.id,
                empty.first(), empty.exists(), empty.count(), empty.nearest(), empty.nearestPreferLocal(2)],
            nameEdges: [matchesEntityName(null, 'x'), matchesEntityName(' Goblin ', 'goblin'),
                matchesEntityName(7, 7),
                EntityQuery.fromSnapshots(() => [{ x: 0, z: 0, distance: 0 }], (row) => row)
                    .name('undefined').count()],
            invalid: invalid.map((row) => row.name),
            supplyCalls: supplies,
            identity: [first === wrapped[3], results[1] === wrapped[1]],
            order,
            callbackResult,
            early: [stopped, iteratorReturn, iteratorNext],
            callbackErrors,
            overrides: [preferred, resultCalls, overriddenForEachThis, overrideResults[0] === earlyEntity],
            radii: radiusResults,
            actionPositions,
            mutable: [mutableWinner.id, mutableCalls],
        };
    }
}
"#;

fn actor_snapshot<'a>(
    tick: u64,
    npcs: &'a [SceneEntityInput<'a>],
    locs: &'a [SceneEntityInput<'a>],
    players: &'a [SceneEntityInput<'a>],
    ground: &'a [SceneEntityInput<'a>],
) -> SnapshotInput<'a> {
    let mut snapshot = empty_snapshot(tick);
    snapshot.npcs = npcs;
    snapshot.locs = locs;
    snapshot.players = players;
    snapshot.ground = ground;
    snapshot
}

fn query_batch_tick(iso: &LoadIsolate, tick: u64, iterations: u32) -> serde_json::Value {
    iso.probe(&format!("globalThis.__queryIterations = {iterations}"))
        .unwrap();
    let mut snapshot = empty_snapshot(tick);
    let actions = ["Attack".to_string()];
    let npcs = [entity(1, 9, Some("Goblin"), 3201, 3201, 0, 1, &actions)];
    snapshot.npcs = &npcs;
    iso.post_snapshot(encode_snapshot(&snapshot));
    iso.on_game_tick(tick);
    iso.probe("__queryResult").unwrap()
}

const QUERY_BATCH_API: &str = r#"
import { Npcs } from '../../api/npcs/Npcs.js';
export default class AllocationBot extends LoopingBot {
    loop() {
        let total = 0;
        for (let i = 0; i < globalThis.__queryIterations; i++) {
            total += Npcs.query().name('Goblin').action('Attack').count();
        }
        globalThis.__queryResult = total;
    }
}
"#;

#[test]
fn complete_entities_and_query_contract_on_served_isolates() {
    let npc_actions = [
        "hidden".to_string(),
        "Attack".to_string(),
        "Talk-to".to_string(),
    ];
    let loc_actions = [
        "hidden".to_string(),
        "Open".to_string(),
        "Search".to_string(),
    ];
    let player_actions = ["hidden".to_string(), "Trade".to_string()];
    let ground_actions = ["hidden".to_string(), "Take".to_string()];
    let mut npc = entity(7, 9, Some("Goblin"), 3201, 3202, 0, 1, &npc_actions);
    npc.in_combat = true;
    npc.health = 5;
    npc.max_health = 10;
    npc.combat_level = 8;
    npc.target_kind = 2;
    npc.target_index = 2;
    npc.size = 2;
    npc.nx = 3199;
    npc.nz = 3200;
    let mut other_npc = entity(8, 10, Some("Goblin"), 3202, 3201, 1, 2, &npc_actions);
    other_npc.target_kind = 2;
    other_npc.target_index = 3;
    let npcs = [npc, other_npc];
    let locs = [entity(1, 100, Some("Door"), 3200, 3201, 0, 1, &loc_actions)];
    let mut player = entity(2, 20, Some("alice"), 3202, 3202, 0, 2, &player_actions);
    player.in_combat = true;
    player.combat_level = 42;
    player.target_kind = 2;
    player.target_index = 2;
    let players = [player];
    let ground = [entity(
        3,
        200,
        Some("Bones"),
        3200,
        3202,
        0,
        2,
        &ground_actions,
    )];
    let snapshot = actor_snapshot(1, &npcs, &locs, &players, &ground);

    let iso = LoadIsolate::spawn(ENTITY_API.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    assert_eq!(iso.probe("1").unwrap(), serde_json::json!(1));
    let probe = tick(&iso, &snapshot);
    assert_eq!(probe["aliases"], serde_json::json!([true, true, true]));
    assert_eq!(probe["roots"], serde_json::json!([2, 1, 2]));
    assert_eq!(probe["wrapped"], serde_json::json!([true, true, true]));
    assert_eq!(probe["npc"][0], "Goblin");
    assert_eq!(
        probe["npc"].as_array().unwrap()[1..7],
        serde_json::json!([9, 7, true, 5, 8, 2]).as_array().unwrap()[..]
    );
    assert_eq!(
        probe["npc"].as_array().unwrap()[7..9],
        serde_json::json!([3201, 3202]).as_array().unwrap()[..]
    );
    assert_eq!(probe["npc"][9], 1);
    assert_eq!(probe["npc"][10], serde_json::json!(["Attack", "Talk-to"]));
    assert_eq!(
        probe["npc"].as_array().unwrap()[11..13],
        serde_json::json!([3199, 3200]).as_array().unwrap()[..]
    );
    assert_eq!(probe["targetVariants"], serde_json::json!([false, true]));
    assert_eq!(
        probe["loc"].as_array().unwrap()[0..6],
        serde_json::json!(["Door", 100, 3200, 3201, 1, ["Open", "Search"]])
            .as_array()
            .unwrap()[..]
    );
    assert_eq!(
        probe["player"].as_array().unwrap()[0..4],
        serde_json::json!(["alice", 2, true, 42])
            .as_array()
            .unwrap()[..]
    );
    assert_eq!(probe["player"][8], true);
    assert_eq!(
        probe["ground"].as_array().unwrap()[0..7],
        serde_json::json!(["Bones", 200, 1, 3200, 3202, 2, ["Take"]])
            .as_array()
            .unwrap()[..]
    );
    assert_eq!(
        probe["partial"],
        serde_json::json!([null, null, null, [], 0])
    );
    assert_eq!(
        probe["partialOther"],
        serde_json::json!([null, null, 0, [], null, null, 0, [], null, null, 1, 0, [],])
    );
    assert_eq!(
        probe["support"],
        serde_json::json!(["Talk-to", true, false, true])
    );
    assert_eq!(
        probe["queueResults"],
        serde_json::json!([true, true, true, true])
    );
    assert_eq!(probe["nearest"], serde_json::json!([9, 9]));
    assert_eq!(probe["firstIdentity"], true);
    assert_eq!(probe["queryMethods"], serde_json::json!([2, true, true]));
    let interactions = iso.drain_interacts();
    assert_eq!(
        interactions,
        vec![
            InteractReq::Npc {
                name: "Goblin".into(),
                action: "Attack".into(),
                index: Some(7)
            },
            InteractReq::Loc {
                x: 3200,
                z: 3201,
                level: 0,
                action: "Open".into(),
                id: Some(100)
            },
            InteractReq::Player {
                name: "alice".into(),
                action: "Trade".into()
            },
            InteractReq::Obj {
                x: 3200,
                z: 3202,
                level: 0,
                name: Some("Bones".into()),
                action: "Take".into()
            },
        ]
    );
    iso.join();

    let query_iso =
        LoadIsolate::spawn(QUERY_API.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    assert_eq!(query_iso.probe("1").unwrap(), serde_json::json!(1));
    let query_probe = tick(&query_iso, &empty_snapshot(1));
    assert_eq!(query_probe["chain"][0], serde_json::json!([1, 2, 3]));
    assert_eq!(query_probe["chain"][1], 1);
    assert_eq!(query_probe["chain"][2], 2);
    assert_eq!(
        query_probe["chain"].as_array().unwrap()[3..8],
        serde_json::json!([null, false, 0, null, null])
            .as_array()
            .unwrap()[..]
    );
    assert_eq!(
        query_probe["nameEdges"],
        serde_json::json!([false, true, true, 1])
    );
    assert_eq!(query_probe["invalid"], serde_json::json!(["partial"]));
    assert_eq!(query_probe["supplyCalls"], 3);
    assert_eq!(query_probe["identity"], serde_json::json!([true, true]));
    assert_eq!(query_probe["callbackResult"], true);
    assert_eq!(
        query_probe["callbackErrors"],
        serde_json::json!([true, true, true, true, true])
    );
    assert_eq!(query_probe["early"][0], true);
    assert_eq!(query_probe["early"][1], 1);
    assert_eq!(query_probe["early"][2], 1);
    assert_eq!(
        query_probe["overrides"],
        serde_json::json!([true, 1, true, true])
    );
    assert_eq!(
        query_probe["radii"],
        serde_json::json!(["near", "near", "near", "near", "near"])
    );
    assert_eq!(
        query_probe["actionPositions"],
        serde_json::json!([3, ["Attack"]])
    );
    assert_eq!(
        query_probe["mutable"],
        serde_json::json!(["tie", ["far:1", "near:1", "tie:1", "tie:2", "near:2"]])
    );
    let order = query_probe["order"].as_array().unwrap();
    let position = |label: &str| order.iter().position(|value| value == label).unwrap();
    assert!(position("supply-this") < position("get-name"));
    assert!(position("get-name") < position("extra-snap-filter"));
    assert!(position("extra-snap-filter") < position("wrap-this"));
    assert!(position("wrap-this") < position("filter-undefined-this"));
    assert!(position("filter-undefined-this") < position("extra-entity-filter"));
    assert!(position("extra-entity-filter") < position("seen-undefined-this"));
    assert_eq!(order.iter().filter(|value| *value == "get-x").count(), 2);
    query_iso.join();

    // Allocation accounting belongs in the native raw-scope unit tests. A
    // LoadIsolate probe also allocates its transport/result on another thread.
    // Keep this integration fixture as a successful served batching proof.
    let batch_iso =
        LoadIsolate::spawn(QUERY_BATCH_API.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    assert_eq!(batch_iso.probe("1").unwrap(), serde_json::json!(1));
    assert_eq!(query_batch_tick(&batch_iso, 1, 0), serde_json::json!(0));
    assert_eq!(query_batch_tick(&batch_iso, 2, 1), serde_json::json!(1));
    for tick in 3..13 {
        assert_eq!(
            query_batch_tick(&batch_iso, tick, 1_000),
            serde_json::json!(1_000)
        );
    }
    batch_iso.join();
}
