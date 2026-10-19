use script::isolate_fb::{encode_snapshot, ReachViewInput, SceneEntityInput, SnapshotInput, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

struct CountingAllocator;

static ALLOCATION_EVENTS: AtomicU64 = AtomicU64::new(0);
static LIVE_BLOCKS: AtomicU64 = AtomicU64::new(0);
static PEAK_BLOCKS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
static DEALLOCATION_EVENTS: AtomicU64 = AtomicU64::new(0);

fn update_peak(peak: &AtomicU64, value: u64) {
    let mut old = peak.load(Ordering::Relaxed);
    while value > old {
        match peak.compare_exchange_weak(old, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(current) => old = current,
        }
    }
}

fn record_alloc(size: usize) {
    ALLOCATION_EVENTS.fetch_add(1, Ordering::Relaxed);
    ALLOCATED_BYTES.fetch_add(size as u64, Ordering::Relaxed);
    let blocks = LIVE_BLOCKS.fetch_add(1, Ordering::Relaxed) + 1;
    let bytes = LIVE_BYTES.fetch_add(size as u64, Ordering::Relaxed) + size as u64;
    update_peak(&PEAK_BLOCKS, blocks);
    update_peak(&PEAK_BYTES, bytes);
}

// SAFETY: Every pointer and layout passes unchanged to System, which owns all returned blocks. The atomics do not dereference or retain allocation pointers.
unsafe impl GlobalAlloc for CountingAllocator {
    // SAFETY: GlobalAlloc callers provide a valid allocation layout, and System receives that exact layout.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            record_alloc(layout.size());
        }
        ptr
    }

    // SAFETY: GlobalAlloc callers provide a valid allocation layout, and System receives that exact layout.
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc_zeroed(layout);
        if !ptr.is_null() {
            record_alloc(layout.size());
        }
        ptr
    }

    // SAFETY: GlobalAlloc callers provide a live System block and its original layout, and System receives both unchanged.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let resized = System.realloc(ptr, layout, new_size);
        if !resized.is_null() {
            ALLOCATION_EVENTS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
            if new_size >= layout.size() {
                let bytes = LIVE_BYTES.fetch_add((new_size - layout.size()) as u64, Ordering::Relaxed)
                    + (new_size - layout.size()) as u64;
                update_peak(&PEAK_BYTES, bytes);
            } else {
                LIVE_BYTES.fetch_sub((layout.size() - new_size) as u64, Ordering::Relaxed);
            }
        }
        resized
    }

    // SAFETY: GlobalAlloc callers provide a live System block and its original layout, and System receives both unchanged.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        DEALLOCATION_EVENTS.fetch_add(1, Ordering::Relaxed);
        LIVE_BLOCKS.fetch_sub(1, Ordering::Relaxed);
        LIVE_BYTES.fetch_sub(layout.size() as u64, Ordering::Relaxed);
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn allocator_counts_allocation_lifetimes() {
    let before = totals();
    let mut bytes = Vec::with_capacity(1);
    bytes.push(1_u8);
    bytes.reserve(128);
    let during = totals();
    assert!(during.events > before.events);
    assert!(during.allocated_bytes > before.allocated_bytes);
    assert!(during.live_blocks > before.live_blocks);
    let deallocations = DEALLOCATION_EVENTS.load(Ordering::Relaxed);
    drop(bytes);
    assert!(DEALLOCATION_EVENTS.load(Ordering::Relaxed) > deallocations);
    let after = totals();
    assert_eq!(after.live_blocks, before.live_blocks);
    assert_eq!(after.live_bytes, before.live_bytes);

}

#[derive(Clone, Copy, Debug, Default)]
struct Totals {
    events: u64,
    live_blocks: u64,
    peak_blocks: u64,
    allocated_bytes: u64,
    live_bytes: u64,
    peak_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Metrics {
    count_total: u64,
    count_current: i64,
    count_max: u64,
    bytes_total: u64,
    bytes_current: i64,
    bytes_max: u64,
}

fn totals() -> Totals {
    Totals {
        events: ALLOCATION_EVENTS.load(Ordering::Relaxed),
        live_blocks: LIVE_BLOCKS.load(Ordering::Relaxed),
        peak_blocks: PEAK_BLOCKS.load(Ordering::Relaxed),
        allocated_bytes: ALLOCATED_BYTES.load(Ordering::Relaxed),
        live_bytes: LIVE_BYTES.load(Ordering::Relaxed),
        peak_bytes: PEAK_BYTES.load(Ordering::Relaxed),
    }
}

fn measure<T>(operation: impl FnOnce() -> T) -> (T, Metrics) {
    let before = totals();
    PEAK_BLOCKS.store(before.live_blocks, Ordering::Relaxed);
    PEAK_BYTES.store(before.live_bytes, Ordering::Relaxed);
    let result = operation();
    let after = totals();
    let peak_blocks = PEAK_BLOCKS.load(Ordering::Relaxed);
    let peak_bytes = PEAK_BYTES.load(Ordering::Relaxed);
    (
        result,
        Metrics {
            count_total: after.events.saturating_sub(before.events),
            count_current: after.live_blocks as i64 - before.live_blocks as i64,
            count_max: peak_blocks.saturating_sub(before.live_blocks),
            bytes_total: after.allocated_bytes.saturating_sub(before.allocated_bytes),
            bytes_current: after.live_bytes as i64 - before.live_bytes as i64,
            bytes_max: peak_bytes.saturating_sub(before.live_bytes),
        },
    )
}

fn excess(current: Metrics, baseline: Metrics) -> Metrics {
    Metrics {
        count_total: current.count_total.saturating_sub(baseline.count_total),
        count_current: current.count_current.saturating_sub(baseline.count_current),
        count_max: current.count_max.saturating_sub(baseline.count_max),
        bytes_total: current.bytes_total.saturating_sub(baseline.bytes_total),
        bytes_current: current.bytes_current.saturating_sub(baseline.bytes_current),
        bytes_max: current.bytes_max.saturating_sub(baseline.bytes_max),
    }
}

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
            identity: [first === wrapped[3], results[1] === wrapped[5]],
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

fn actor_snapshot<'a>(tick: u64, npcs: &'a [SceneEntityInput<'a>], locs: &'a [SceneEntityInput<'a>],
    players: &'a [SceneEntityInput<'a>], ground: &'a [SceneEntityInput<'a>]) -> SnapshotInput<'a> {
    let mut snapshot = empty_snapshot(tick);
    snapshot.npcs = npcs;
    snapshot.locs = locs;
    snapshot.players = players;
    snapshot.ground = ground;
    snapshot
}

fn allocation_tick(iso: &LoadIsolate, tick: u64, iterations: u32) -> (serde_json::Value, Metrics) {
    iso.probe(&format!("globalThis.__queryIterations = {iterations}")).unwrap();
    let mut snapshot = empty_snapshot(tick);
    let actions = ["Attack".to_string()];
    let npcs = [entity(1, 9, Some("Goblin"), 3201, 3201, 0, 1, &actions)];
    snapshot.npcs = &npcs;
    iso.post_snapshot(encode_snapshot(&snapshot));
    let (probe, metrics) = measure(|| {
        iso.on_game_tick(tick);
        iso.probe("__queryResult").unwrap()
    });
    (probe, metrics)
}

const ALLOCATION_API: &str = r#"
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
    allocator_counts_allocation_lifetimes();
    let npc_actions = ["hidden".to_string(), "Attack".to_string(), "Talk-to".to_string()];
    let loc_actions = ["hidden".to_string(), "Open".to_string(), "Search".to_string()];
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
    let ground = [entity(3, 200, Some("Bones"), 3200, 3202, 0, 2, &ground_actions)];
    let snapshot = actor_snapshot(1, &npcs, &locs, &players, &ground);

    let iso = LoadIsolate::spawn(ENTITY_API.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    assert_eq!(iso.probe("1").unwrap(), serde_json::json!(1));
    let probe = tick(&iso, &snapshot);
    assert_eq!(probe["aliases"], serde_json::json!([true, true, true]));
    assert_eq!(probe["roots"], serde_json::json!([2, 1, 2]));
    assert_eq!(probe["wrapped"], serde_json::json!([true, true, true]));
    assert_eq!(probe["npc"][0], "Goblin");
    assert_eq!(probe["npc"][1..7], serde_json::json!([9, 7, true, 5, 8, 2])[..]);
    assert_eq!(probe["npc"][7..9], serde_json::json!([3201, 3202])[..]);
    assert_eq!(probe["npc"][9], 1);
    assert_eq!(probe["npc"][10], serde_json::json!(["Attack", "Talk-to"]));
    assert_eq!(probe["npc"][11..13], serde_json::json!([3199, 3200])[..]);
    assert_eq!(probe["targetVariants"], serde_json::json!([false, true]));
    assert_eq!(probe["loc"][0..6], serde_json::json!(
        ["Door", 100, 3200, 3201, 1, ["Open", "Search"]]
    )[..]);
    assert_eq!(probe["player"][0..4], serde_json::json!(["alice", 2, true, 42])[..]);
    assert_eq!(probe["player"][8], true);
    assert_eq!(probe["ground"][0..7], serde_json::json!(
        ["Bones", 200, 1, 3200, 3202, 2, ["Take"]]
    )[..]);
    assert_eq!(probe["partial"], serde_json::json!([null, null, null, [], 0]));
    assert_eq!(probe["partialOther"], serde_json::json!([
        null, null, 0, [],
        null, null, 0, [],
        null, null, 1, 0, [],
    ]));
    assert_eq!(probe["support"], serde_json::json!(
        ["Talk-to", true, false, true]
    ));
    assert_eq!(probe["queueResults"], serde_json::json!([true, true, true, true]));
    assert_eq!(probe["nearest"], serde_json::json!([9, 9]));
    assert_eq!(probe["firstIdentity"], true);
    assert_eq!(probe["queryMethods"], serde_json::json!([2, true, true]));
    let interactions = iso.drain_interacts();
    assert_eq!(interactions, vec![
        InteractReq::Npc { name: "Goblin".into(), action: "Attack".into(), index: Some(7) },
        InteractReq::Loc { x: 3200, z: 3201, level: 0, action: "Open".into(), id: Some(100) },
        InteractReq::Player { name: "alice".into(), action: "Trade".into() },
        InteractReq::Obj { x: 3200, z: 3202, level: 0, name: Some("Bones".into()), action: "Take".into() },
    ]);
    iso.join();

    let query_iso = LoadIsolate::spawn(QUERY_API.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    assert_eq!(query_iso.probe("1").unwrap(), serde_json::json!(1));
    let query_probe = tick(&query_iso, &empty_snapshot(1));
    assert_eq!(query_probe["chain"][0], serde_json::json!([1, 2, 3]));
    assert_eq!(query_probe["chain"][1], 1);
    assert_eq!(query_probe["chain"][2], 2);
    assert_eq!(query_probe["chain"][3..8], serde_json::json!([null, false, 0, null, null])[..]);
    assert_eq!(query_probe["nameEdges"], serde_json::json!([false, true, true, 1]));
    assert_eq!(query_probe["invalid"], serde_json::json!(["partial"]));
    assert_eq!(query_probe["supplyCalls"], 3);
    assert_eq!(query_probe["identity"], serde_json::json!([true, true]));
    assert_eq!(query_probe["callbackResult"], true);
    assert_eq!(query_probe["callbackErrors"], serde_json::json!([true, true, true, true, true]));
    assert_eq!(query_probe["early"][0], true);
    assert_eq!(query_probe["early"][1], 1);
    assert_eq!(query_probe["early"][2], 1);
    assert_eq!(query_probe["overrides"], serde_json::json!([true, 1, true, true]));
    assert_eq!(query_probe["radii"], serde_json::json!(
        ["near", "near", "near", "near", "near"]
    ));
    assert_eq!(query_probe["actionPositions"], serde_json::json!([3, ["Attack"]]));
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

    let allocation_iso = LoadIsolate::spawn(ALLOCATION_API.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    assert_eq!(allocation_iso.probe("1").unwrap(), serde_json::json!(1));
    let (_, cold_control) = allocation_tick(&allocation_iso, 1, 0);
    let (cold_probe, cold_query) = allocation_tick(&allocation_iso, 2, 1);
    assert_eq!(cold_probe, serde_json::json!(1));
    let mut tick_id = 3;
    for _ in 0..5 {
        let (_, _) = allocation_tick(&allocation_iso, tick_id, 1_000);
        tick_id += 1;
    }
    let mut warmed_control = Metrics {
        count_total: u64::MAX,
        count_current: i64::MAX,
        count_max: u64::MAX,
        bytes_total: u64::MAX,
        bytes_current: i64::MAX,
        bytes_max: u64::MAX,
    };
    let mut warmed_query = warmed_control;
    for _ in 0..5 {
        let (_, control) = allocation_tick(&allocation_iso, tick_id, 0);
        tick_id += 1;
        let (result, query) = allocation_tick(&allocation_iso, tick_id, 1_000);
        tick_id += 1;
        assert_eq!(result, serde_json::json!(1_000));
        if control.count_total < warmed_control.count_total {
            warmed_control = control;
        }
        if query.count_total < warmed_query.count_total {
            warmed_query = query;
        }
    }
    let warm_excess = excess(warmed_query, warmed_control);
    let cold_query_excess = excess(cold_query, cold_control);
    eprintln!("entity query allocation metrics: cold_control={cold_control:?}; cold_first_touch={cold_query:?}; cold_query_excess={cold_query_excess:?}; warm_control={warmed_control:?}; warm_excess={warm_excess:?}");
    assert!(cold_query.count_total <= 3, "cold entity callback allocation events: {cold_query:?}");
    assert!(cold_query.count_current <= 3, "cold entity callback retained blocks: {cold_query:?}");
    assert!(cold_query.count_max <= 3, "cold entity callback peak blocks: {cold_query:?}");
    assert!(cold_query.bytes_total <= 336, "cold entity callback allocation bytes: {cold_query:?}");
    assert!(cold_query.bytes_current <= 336, "cold entity callback retained bytes: {cold_query:?}");
    assert!(cold_query.bytes_max <= 336, "cold entity callback peak bytes: {cold_query:?}");
    assert_eq!(warm_excess, Metrics::default(), "warmed query callbacks added Rust allocations over the control");
    allocation_iso.join();
}
