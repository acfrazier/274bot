//! Public-path proof for the marshal-only SolveClue adapter. The catalog
//! embeds construct it in `onStart` and let `TaskBot.loop` drive
//! `validate` / `execute` on every tick, so these tests drive that same
//! shape — the clue task beside a sibling grind task, over a real
//! selected-revision isolate and the real posted snapshot pages. The adapter
//! owns no sequencing, so every assertion is about what the isolate machine
//! answered and what reached the interact drain.
use client::io::ClientRevision;
use script::isolate_fb::{ItemRowInput, SceneEntityInput, SnapshotInput, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

/// `trail_clue_easy_simple001`: a selected search membership whose
/// `trail_coord` decodes to `(3209, 3218, 1)`.
const SEARCH_ID: i32 = 2677;
/// The packed `access: "constrained"` row: refused with `aborted` /
/// `constrained`, never a token and never a verb.
const CONSTRAINED_ID: i32 = 3554;
/// The casket the constrained row's own selected `trail_casket` names.
const CASKET_ID: i32 = 3555;
/// `trail_clue_medium_sextant001`: the unguarded-dig membership whose
/// `trail_coord` decodes to `(3160, 3251, 0)`.
const UNGUARDED_ID: i32 = 2801;
/// The item the frozen `Spade` display belongs to: the pack row the Dig
/// resolves by posted name.
const SPADE_ITEM: i32 = 952;
/// The three selected coordinate-tool items the acquire chain joins by id, and
/// the display names their posted pack rows carry. A scene that posts no trio
/// is a scene the acquire chain owns rather than the dig arm the test drives.
const SEXTANT_ITEM: i32 = 2574;
const WATCH_ITEM: i32 = 2575;
const CHART_ITEM: i32 = 2576;
const TRIO_NAMES: [(i32, &str); 3] = [
    (SEXTANT_ITEM, "Sextant"),
    (WATCH_ITEM, "Watch"),
    (CHART_ITEM, "Chart"),
];
/// The decoded tile `SEARCH_ID` walks to.
const SEARCH_X: i32 = 3209;
const SEARCH_Z: i32 = 3218;
const SEARCH_LEVEL: i32 = 1;

/// `trail_clue_hard_riddle027`: the selected Entrana-box membership, whose
/// `trail_coord=0_44_52_2_23` decodes to `(2818, 3351, 0)` inside the cap box.
const ENTRANA_ID: i32 = 3579;
const ENTRANA_X: i32 = 2818;
const ENTRANA_Z: i32 = 3351;
/// The refused worn name the frozen matcher folds, and the hard-trail dagger
/// id the strip unequips but never lists.
const HELM_ITEM: i32 = 1163;
const DDS_ITEM: i32 = 1231;
/// A worn name the frozen matcher lets through: never a verb, never listed.
const GLORY_ITEM: i32 = 1704;

/// The posted pages these tests need: the pack page is the `page` argument,
/// plus this call's `here` tile, its posted loc page, its posted stat page,
/// the posted display names the pack rows carry, the posted main modal and the
/// posted `hold || ours` pair.
struct Scene<'a> {
    here: Option<TileInput>,
    locs: &'a [SceneEntityInput<'a>],
    /// The display names the posted pack rows carry, by item id: the Drop, the
    /// Dig and the Open all resolve identity by posted name.
    names: &'a [(i32, &'a str)],
    /// The posted stat page the death read comes off.
    stats: &'a [script::isolate_fb::StatInput<'a>],
    main_modal_id: i32,
    hold: bool,
    ours: bool,
    /// The posted npc page the talk arm reads.
    npcs: &'a [SceneEntityInput<'a>],
    /// The posted chat modal: `-1` is the closed one the page posts itself.
    chat_modal_id: i32,
    /// The posted `chat_continue`.
    chat_continue: bool,
    /// The posted count dialog the challenge step's answer rides.
    count_dialog_open: bool,
    /// The posted chat choices the trio acquire chain reads, in posted order:
    /// each row's own 1-based slot is its position here.
    options: &'a [script::isolate_fb::ChatOptionInput<'a>],
    /// Whether the posted pack carries the held coordinate trio: a dig-row
    /// scene needs it, because the acquire chain owns the row until it does.
    trio: bool,
    /// The posted worn page the Entrana strip reads: the raw
    /// `host().snapshot.equipment` rows with their own slot.
    equipment: &'a [ItemRowInput<'a>],
    /// The posted nearest Use-quickly booth the strip's and the restore's bank
    /// trip opens.
    nearest_booth: Option<&'a script::isolate_fb::NearestBoothInput<'a>>,
    /// The posted bank interface: the strip's deposit and the restore's claim
    /// go out only behind a posted open one.
    bank_open: bool,
}

impl Default for Scene<'_> {
    fn default() -> Self {
        Self {
            here: None,
            locs: &[],
            names: &[],
            stats: &[],
            main_modal_id: 0,
            hold: false,
            ours: false,
            npcs: &[],
            chat_modal_id: -1,
            chat_continue: false,
            count_dialog_open: false,
            options: &[],
            trio: false,
            equipment: &[],
            nearest_booth: None,
            bank_open: false,
        }
    }
}

/// One posted worn row: the name the frozen matcher folds, the packed id its
/// two hard-trail dagger ids are joined by, and the slot the raw page carries.
fn worn_row(id: i32, name: &str, slot: i32) -> ItemRowInput<'_> {
    ItemRowInput {
        name: Some(name),
        count: 1,
        id,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot,
    }
}

/// The posted nearest Use-quickly booth the Entrana bank trip opens: the tile
/// beside the player, the loc's own id and the two labels the landed bank
/// helpers queue on `open-booth`.
const POSTED_BOOTH: script::isolate_fb::NearestBoothInput<'static> =
    script::isolate_fb::NearestBoothInput {
        x: 2810,
        z: 3350,
        level: 0,
        id: 2213,
        name: "Bank booth",
        op: "Use-quickly",
    };

fn post_page(iso: &LoadIsolate, tick: u64, page: &[(i32, i32)]) {
    post_scene(iso, tick, page, &Scene::default());
}

fn post_scene(iso: &LoadIsolate, tick: u64, page: &[(i32, i32)], scene: &Scene<'_>) {
    let mut held: Vec<(i32, i32)> = page.to_vec();
    if scene.trio {
        held.extend(TRIO_NAMES.iter().map(|(id, _)| (*id, 1)));
    }
    let rows: Vec<ItemRowInput<'_>> = held
        .iter()
        .enumerate()
        .map(|(slot, (id, count))| ItemRowInput {
            name: scene
                .names
                .iter()
                .find(|(item, _)| item == id)
                .map(|(_, name)| *name)
                .or_else(|| {
                    TRIO_NAMES
                        .iter()
                        .find(|(item, _)| item == id)
                        .map(|(_, name)| *name)
                }),
            count: *count,
            id: *id,
            ops: &[],
            noted: false,
            cert: -1,
            component_id: -1,
            slot: slot as i32,
        })
        .collect();
    let input = SnapshotInput {
        tick,
        here: scene.here,
        ingame: true,
        inv: &rows,
        inv_size: 28,
        stats: scene.stats,
        booths: &[],
        nearest_booth: scene
            .nearest_booth
            .map(|booth| script::isolate_fb::NearestBoothInput {
                x: booth.x,
                z: booth.z,
                level: booth.level,
                id: booth.id,
                name: booth.name,
                op: booth.op,
            }),
        banks: &[],
        bank: &[],
        bank_side: &[],
        bank_open: scene.bank_open,
        bank_loaded: false,
        bank_generation: 0,
        count_dialog_open: scene.count_dialog_open,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
        withdraw_load_result_seq: 0,
        withdraw_load_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
        hold: scene.hold,
        ours: scene.ours,
        npcs: scene.npcs,
        locs: scene.locs,
        players: &[],
        ground: &[],
        equipment: scene.equipment,
        chat_open: false,
        chat_continue: scene.chat_continue,
        chat_text: None,
        chat_options: scene.options,
        side_tab: -1,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: None,
        in_combat: false,
        animating: false,
        main_modal_id: scene.main_modal_id,
        chat_modal_id: scene.chat_modal_id,
        make_products: &[],
        side_tab_ifaces: &[],
        spell_buttons: &[],
        chat_lines: &[],
        bank_note_on: -1,
        bank_note_off: -1,
        scene_state: 0,
        weight: 0,
        combat_level: 0,
        camera_yaw: 0,
        camera_pitch: 0,
        teleports_enabled: false,
        self_slot: 0,
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
        reach: script::isolate_fb::ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
    };
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&input));
}

/// One posted loc row: the fields the frozen picker reads.
fn scene_loc<'a>(
    id: i32,
    x: i32,
    z: i32,
    level: i32,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: 7,
        id,
        name: None,
        x,
        z,
        level,
        distance: 1,
        health: 0,
        max_health: 0,
        in_combat: false,
        animating: false,
        actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 1,
        nx: x,
        nz: z,
    }
}

fn spawn(src: &str) -> LoadIsolate {
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    LoadIsolate::spawn_with_game_data(src.to_string(), LoadShape::CompatClass, vec![], data)
        .unwrap()
}

/// The walked step the search membership's decoded tile is: the select
/// defaults are the ones an absent option takes on the isolate wire.
fn walk_to(x: i32, z: i32, level: i32) -> InteractReq {
    InteractReq::Walk {
        x,
        z,
        level,
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
        request_id: 0,
    }
}

fn json(iso: &LoadIsolate, expr: &str) -> serde_json::Value {
    let raw = iso.probe(expr).unwrap();
    serde_json::from_str(raw.as_str().expect("probe JSON string")).unwrap()
}

/// The probe's own scalar text: a `String(...)` expression reads back as the
/// string it produced, anything else as its JSON text.
fn probe_text(iso: &LoadIsolate, expr: &str) -> String {
    match iso.probe(expr).unwrap() {
        serde_json::Value::String(text) => text,
        value => value.to_string(),
    }
}

/// A tick's barrier: `on_game_tick` is fire-and-forget, so a probe proves the
/// tick's JS ran and its interact batch was forwarded.
fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    assert!(iso.probe("true").is_ok());
}

/// The isolate machine's own answer to a `next` on the token the task is
/// driving, called straight through the registered helper: the machine decides
/// whether that token is live, so the adapter cannot fake it. A task that
/// never began holds no token, and the machine answers the refusal.
fn machine_next(iso: &LoadIsolate, hold: bool) -> serde_json::Value {
    json(
        iso,
        &format!(
            "JSON.stringify(globalThis.rustyscript.functions.__rs2b0t_clue({{\
             op: 'next', token: globalThis.__rs_bot.solveClue.token, generation: 0, \
             hold: {hold}, held: [[{SEARCH_ID}, 1]] }}))"
        ),
    )
}

fn assert_clean(logs: &[String]) {
    assert!(logs.is_empty(), "the adapter must not throw: {logs:?}");
}

/// The idle ABI: construct, read status, own nothing, latch nothing. The
/// disabled gate is the whole session check, so no callback and no verb may
/// ride along even with a scroll held.
#[test]
fn solve_clue_adapter_surface_is_idle_and_owns_no_equipment() {
    let src = r#"
import * as clueModule from '../../api/ai/clues/SolveClue.js';
import { SolveClue, heldClueLikeId, walkToBank } from '../../api/ai/clues/SolveClue.js';
export default class T extends TaskBot {
    onStart() {
        globalThis.__reached = [];
        this.solveClue = new SolveClue({
            enabled: () => false,
            log: (message) => { globalThis.__reached.push('log:' + message); },
            setStatus: (message) => { globalThis.__reached.push('setStatus:' + message); },
            isFood: (name) => { globalThis.__reached.push('isFood:' + name); return false; },
            foodName: () => { globalThis.__reached.push('foodName'); return 'Lobster'; },
            foodWithdraw: () => { globalThis.__reached.push('foodWithdraw'); return 0; },
            weaponName: () => { globalThis.__reached.push('weaponName'); return 'Rune scimitar'; },
        });
        this.add(this.solveClue);
        const thrown = (fn) => {
            try { fn(); return 'no throw'; } catch (e) { return String(e && (e.message || e)); }
        };
        globalThis.__probe = JSON.stringify({
            status: this.solveClue.clueStatus(),
            owns: typeof this.solveClue.ownsEquipment === 'function'
                ? this.solveClue.ownsEquipment()
                : 'missing',
            retry: typeof this.solveClue.retry,
            retried: this.solveClue.retry(),
            note: thrown(() => this.solveClue.noteDeath()),
            heldClue: thrown(() => heldClueLikeId()),
            bank: thrown(() => walkToBank()),
            exports: Object.keys(clueModule).sort().join(','),
            reached: globalThis.__reached,
        });
    }
}
"#;
    let iso = spawn(src);
    post_page(&iso, 1, &[(SEARCH_ID, 1)]);
    tick(&iso, 1);
    let value = json(&iso, "globalThis.__probe");
    assert!(
        iso.drain_interacts().is_empty(),
        "a disabled adapter queues nothing"
    );
    let logs = iso.drain_logs();
    iso.join();
    assert_eq!(value["status"], "idle", "{value:?}");
    assert_eq!(value["owns"], false, "{value:?}");
    assert_eq!(
        value["retry"], "function",
        "the landed `SolveClue.retry()` seat: {value:?}"
    );
    assert_eq!(
        value["retried"], true,
        "the machine's own latch clear answers on an idle isolate too: {value:?}"
    );
    assert_eq!(value["note"], "no throw", "{value:?}");
    assert_eq!(
        value["heldClue"], "not impl: SolveClue.heldClueLikeId",
        "{value:?}"
    );
    assert_eq!(value["bank"], "not impl: SolveClue.walkToBank", "{value:?}");
    assert_eq!(
        value["exports"], "SolveClue,heldClueLikeId,walkToBank",
        "the module surface is the class and the two throwing helpers: {value:?}"
    );
    assert_eq!(
        value["reached"],
        serde_json::json!([]),
        "a disabled session dispatches no callback: {value:?}"
    );
    assert_clean(&logs);
}

/// Disabled is validate-false, not a begin: the held row stays unclaimed, the
/// machine is left idle (a `next` on the token it never issued is stale) and
/// the sibling grind task takes the tick. The same task then runs the moment
/// the embed's gate comes back.
#[test]
fn solve_clue_adapter_does_not_begin_or_steal_the_grind_while_disabled() {
    let src = r#"
import { SolveClue } from '../../api/ai/clues/SolveClue.js';
export default class T extends TaskBot {
    onStart() {
        globalThis.__statuses = [];
        globalThis.__logs = [];
        globalThis.__reached = [];
        this.solveClue = new SolveClue({
            enabled: () => globalThis.__enabled === true,
            log: (message) => { globalThis.__logs.push(String(message)); },
            setStatus: (message) => { globalThis.__statuses.push(String(message)); },
            isFood: (name) => { globalThis.__reached.push('isFood:' + name); return false; },
            foodName: () => { globalThis.__reached.push('foodName'); return 'Lobster'; },
            foodWithdraw: () => { globalThis.__reached.push('foodWithdraw'); return 0; },
            weaponName: () => { globalThis.__reached.push('weaponName'); return 'Rune scimitar'; },
        });
        this.add(this.solveClue, {
            validate: () => true,
            execute: async () => { globalThis.__grind = (globalThis.__grind || 0) + 1; },
        });
    }
}
"#;
    let iso = spawn(src);
    let page = [(SEARCH_ID, 1)];

    post_page(&iso, 1, &page);
    tick(&iso, 1);
    assert!(
        iso.drain_interacts().is_empty(),
        "a disabled clue task queues nothing"
    );
    assert_eq!(
        machine_next(&iso, false)["kind"],
        "aborted",
        "a disabled validate must not begin: the machine has no live token"
    );
    assert_eq!(
        probe_text(&iso, "String(globalThis.__grind)"),
        "1",
        "the sibling grind task takes the tick the clue task refused"
    );

    iso.probe("globalThis.__enabled = true").unwrap();
    post_scene(
        &iso,
        2,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3200,
                z: 3218,
                level: 1,
            }),
            ..Scene::default()
        },
    );
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![walk_to(SEARCH_X, SEARCH_Z, SEARCH_LEVEL)],
        "enabled again, the same task marshals the machine's own walk"
    );
    let value = json(
        &iso,
        "JSON.stringify({ grind: globalThis.__grind, logs: globalThis.__logs, \
          statuses: globalThis.__statuses, reached: globalThis.__reached })",
    );
    let logs = iso.drain_logs();
    iso.join();
    assert_eq!(value["grind"], 1, "{value:?}");
    assert_eq!(value["logs"].as_array().map(Vec::len), Some(1), "{value:?}");
    assert_eq!(
        value["statuses"].as_array().map(Vec::len),
        Some(1),
        "{value:?}"
    );
    assert!(
        !value.to_string().contains("clue solved"),
        "nothing here marks the clue finished: {value:?}"
    );
    assert_clean(&logs);
}

/// The drain the embeds depend on: `TaskBot.validate` begins once,
/// `execute` answers `callback.enabled` with the embed's own gate, the logged
/// progress and status lines reach the host callbacks, and the machine's walk
/// and loc steps reach the interact drain with the landed field names.
#[test]
fn solve_clue_adapter_drains_the_search_step_over_the_task_loop() {
    let src = r#"
import { SolveClue } from '../../api/ai/clues/SolveClue.js';
export default class T extends TaskBot {
    onStart() {
        globalThis.__logs = [];
        globalThis.__statuses = [];
        globalThis.__reached = [];
        this.solveClue = new SolveClue({
            enabled: () => true,
            log: (message) => { globalThis.__logs.push(String(message)); },
            setStatus: (message) => { globalThis.__statuses.push(String(message)); },
            isFood: (name) => { globalThis.__reached.push('isFood:' + name); return false; },
            foodName: () => { globalThis.__reached.push('foodName'); return 'Lobster'; },
            foodWithdraw: () => { globalThis.__reached.push('foodWithdraw'); return 0; },
            weaponName: () => { globalThis.__reached.push('weaponName'); return 'Rune scimitar'; },
        });
        this.add(this.solveClue);
    }
}
"#;
    let iso = spawn(src);
    let page = [(SEARCH_ID, 1)];
    let actions = vec!["Search".to_string()];
    let locs = [scene_loc(25, 3210, 3217, 1, &actions)];

    // Tick 1: identified and reported. A page with no posted `here` is no
    // arrival claim, so no verb rides along.
    post_page(&iso, 1, &page);
    tick(&iso, 1);
    assert!(
        iso.drain_interacts().is_empty(),
        "a missing `here` is a wait, not a walk"
    );

    // Tick 2: posted far from the decoded tile — the machine's own walk.
    post_scene(
        &iso,
        2,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3200,
                z: 3218,
                level: 1,
            }),
            ..Scene::default()
        },
    );
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![walk_to(SEARCH_X, SEARCH_Z, SEARCH_LEVEL)],
        "the walk is the decoded tile with default flags"
    );

    // Tick 3: arrived, with the posted row the picker takes.
    post_scene(
        &iso,
        3,
        &page,
        &Scene {
            here: Some(TileInput {
                x: SEARCH_X,
                z: SEARCH_Z,
                level: SEARCH_LEVEL,
            }),
            locs: &locs,
            ..Scene::default()
        },
    );
    tick(&iso, 3);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Loc {
            x: 3210,
            z: 3217,
            level: 1,
            action: "Search".to_string(),
            id: Some(25),
        }],
        "the pick keeps the posted tile and id"
    );
    let value = json(
        &iso,
        "JSON.stringify({ status: globalThis.__rs_bot.solveClue.clueStatus(), \
          logs: globalThis.__logs, statuses: globalThis.__statuses, \
          reached: globalThis.__reached })",
    );
    let logs = iso.drain_logs();
    iso.join();
    let text = value.to_string();
    assert!(
        text.contains("trail_clue_easy_simple001"),
        "the machine's own progress and status identity reach the host: {value:?}"
    );
    assert_eq!(
        value["statuses"].as_array().map(Vec::len),
        Some(1),
        "{value:?}"
    );
    assert_eq!(value["logs"].as_array().map(Vec::len), Some(1), "{value:?}");
    assert_eq!(
        value["status"], value["statuses"][0],
        "clueStatus is the dispatched status line: {value:?}"
    );
    assert!(
        !text.contains("clue solved"),
        "the adapter marks nothing finished: {value:?}"
    );
    assert_eq!(
        value["reached"],
        serde_json::json!([]),
        "the unposted callback holes are never reached: {value:?}"
    );
    assert_clean(&logs);
}

/// The live-token rules: a cooperative interrupt returns with the token live,
/// the next tick's `validate` reuses that token instead of beginning a second
/// session, and a disabled gate refuses without aborting the session it
/// already has.
#[test]
fn solve_clue_adapter_keeps_a_live_token_and_never_begins_twice() {
    let src = r#"
import { SolveClue } from '../../api/ai/clues/SolveClue.js';
export default class T extends TaskBot {
    onStart() {
        globalThis.__statuses = [];
        this.solveClue = new SolveClue({
            enabled: () => globalThis.__enabled !== false,
            log: () => {},
            setStatus: (message) => { globalThis.__statuses.push(String(message)); },
        });
        this.add(this.solveClue, {
            validate: () => true,
            execute: async () => { globalThis.__grind = (globalThis.__grind || 0) + 1; },
        });
    }
}
"#;
    let iso = spawn(src);
    let page = [(SEARCH_ID, 1)];
    let actions = vec!["Search".to_string()];
    let locs = [scene_loc(25, 3210, 3217, 1, &actions)];

    // Tick 1: the posted interrupt. The session lives, the task returns.
    post_scene(
        &iso,
        1,
        &page,
        &Scene {
            ours: true,
            ..Scene::default()
        },
    );
    tick(&iso, 1);
    assert!(
        iso.drain_interacts().is_empty(),
        "a cooperative interrupt queues no verb"
    );
    assert_eq!(
        probe_text(&iso, "String(globalThis.__rs_bot.solveClue.validate())"),
        "true",
        "a live token validates true"
    );
    assert_eq!(
        machine_next(&iso, true)["kind"],
        "yield",
        "the first begin's token is still the machine's live token"
    );

    // Tick 2: the gate is false while the interrupt is still posted. The task
    // holds a live token, so `validate` answers false and the sibling grind
    // task takes the tick — the session it already has is not aborted.
    iso.probe("globalThis.__enabled = false").unwrap();
    post_scene(
        &iso,
        2,
        &page,
        &Scene {
            ours: true,
            ..Scene::default()
        },
    );
    tick(&iso, 2);
    let queued = iso.drain_interacts();
    assert!(
        queued.is_empty(),
        "a disabled tick queues nothing: {queued:?}"
    );
    assert_eq!(
        probe_text(&iso, "String(globalThis.__rs_bot.solveClue.validate())"),
        "false",
        "the embed's own gate is the validate answer"
    );
    assert_eq!(
        machine_next(&iso, true)["kind"],
        "yield",
        "a disabled validate leaves the live token in place"
    );
    assert_eq!(
        probe_text(&iso, "String(globalThis.__grind)"),
        "1",
        "the sibling grind task takes the disabled tick"
    );

    // Tick 3: enabled again with the interrupt still posted. The same session
    // keeps the token, and still reports nothing.
    iso.probe("globalThis.__enabled = true").unwrap();
    post_scene(
        &iso,
        3,
        &page,
        &Scene {
            ours: true,
            ..Scene::default()
        },
    );
    tick(&iso, 3);
    assert!(
        iso.drain_interacts().is_empty(),
        "a cooperative interrupt queues no verb"
    );

    // Tick 4: the interrupt lifted, and `here` is posted far from the decoded
    // tile. The same session drains — a second begin would have killed its
    // token and re-run the gate callbacks.
    post_scene(
        &iso,
        4,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3200,
                z: 3218,
                level: 1,
            }),
            ..Scene::default()
        },
    );
    tick(&iso, 4);
    assert_eq!(
        iso.drain_interacts(),
        vec![walk_to(SEARCH_X, SEARCH_Z, SEARCH_LEVEL)],
        "the live session resumed on the same token"
    );
    assert_eq!(
        machine_next(&iso, true)["kind"],
        "yield",
        "validate never began again: the token is still live"
    );

    // Tick 5: arrived — the same session keeps draining.
    post_scene(
        &iso,
        5,
        &page,
        &Scene {
            here: Some(TileInput {
                x: SEARCH_X,
                z: SEARCH_Z,
                level: SEARCH_LEVEL,
            }),
            locs: &locs,
            ..Scene::default()
        },
    );
    tick(&iso, 5);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Loc {
            x: 3210,
            z: 3217,
            level: 1,
            action: "Search".to_string(),
            id: Some(25),
        }],
        "the resumed session keeps draining"
    );
    let value = json(
        &iso,
        "JSON.stringify({ statuses: globalThis.__statuses, grind: globalThis.__grind })",
    );
    let logs = iso.drain_logs();
    iso.join();
    assert_eq!(
        value["statuses"].as_array().map(Vec::len),
        Some(1),
        "one session, one gate: a second begin would have reported again: {value:?}"
    );
    assert_eq!(value["grind"], 1, "{value:?}");
    assert!(
        !value.to_string().contains("clue solved"),
        "nothing here marks the clue finished: {value:?}"
    );
    assert_clean(&logs);
}

/// Losing the held row ends the session without a completion kind: the token
/// dies, the adapter queues nothing more, the frozen pack page lets the
/// sibling grind task back in, and the next held row begins a fresh session.
#[test]
fn solve_clue_adapter_finishes_none_held_without_a_solved_mark() {
    let src = r#"
import { SolveClue } from '../../api/ai/clues/SolveClue.js';
export default class T extends TaskBot {
    onStart() {
        globalThis.__statuses = [];
        globalThis.__logs = [];
        this.solveClue = new SolveClue({
            enabled: () => true,
            log: (message) => { globalThis.__logs.push(String(message)); },
            setStatus: (message) => { globalThis.__statuses.push(String(message)); },
        });
        this.add(this.solveClue, {
            validate: () => true,
            execute: async () => { globalThis.__grind = (globalThis.__grind || 0) + 1; },
        });
    }
}
"#;
    let iso = spawn(src);
    let page = [(SEARCH_ID, 1)];

    // Tick 1: the session opens over the held row.
    post_page(&iso, 1, &page);
    tick(&iso, 1);
    assert!(
        iso.drain_interacts().is_empty(),
        "no posted `here`, no verb"
    );

    // Tick 2: the row left the pack. The machine ends the session, and the
    // adapter stops on its own abort.
    post_page(&iso, 2, &[]);
    tick(&iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "a finished session queues nothing"
    );
    assert_eq!(
        machine_next(&iso, false)["kind"],
        "aborted",
        "the token is dead, and the end state is not a continue kind"
    );

    // Tick 3: nothing held. `validate` is false, so the grind task gets the
    // tick instead of a second session.
    post_page(&iso, 3, &[]);
    tick(&iso, 3);
    assert!(
        iso.drain_interacts().is_empty(),
        "nothing to do, nothing queued"
    );
    assert_eq!(
        probe_text(&iso, "String(globalThis.__grind)"),
        "1",
        "the finished clue task gives the tick back"
    );

    // Tick 4: a scroll is held again — a fresh session, never a resumed dead
    // one.
    post_page(&iso, 4, &page);
    tick(&iso, 4);
    let value = json(
        &iso,
        "JSON.stringify({ statuses: globalThis.__statuses, logs: globalThis.__logs, \
          grind: globalThis.__grind, status: globalThis.__rs_bot.solveClue.clueStatus() })",
    );
    let logs = iso.drain_logs();
    iso.join();
    assert_eq!(
        value["statuses"].as_array().map(Vec::len),
        Some(2),
        "the held row begins a second session: {value:?}"
    );
    assert_eq!(value["grind"], 1, "{value:?}");
    let text = value.to_string();
    assert!(
        text.contains("trail_clue_easy_simple001"),
        "the second session reports the same identity: {value:?}"
    );
    assert!(
        !text.contains("clue solved"),
        "a finished session is never a solved mark: {value:?}"
    );
    assert_clean(&logs);
}

/// The held casket's own Open: the machine's `held` step reaches the drain as
/// a held-item interaction, and the constrained clue beside it is never
/// played as a loc.
#[test]
fn solve_clue_adapter_opens_the_held_casket_over_the_queue() {
    let src = r#"
import { SolveClue } from '../../api/ai/clues/SolveClue.js';
export default class T extends TaskBot {
    onStart() {
        this.solveClue = new SolveClue({
            enabled: () => true,
            log: () => {},
            setStatus: () => {},
        });
        this.add(this.solveClue);
    }
}
"#;
    let iso = spawn(src);
    let pair = [(CONSTRAINED_ID, 1), (CASKET_ID, 1)];

    post_page(&iso, 1, &pair);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Held {
            name: "Casket".to_string(),
            action: "Open".to_string(),
        }],
        "the casket Open is a held-item request"
    );

    // The casket gone: the constrained clue is refused instead — `aborted` /
    // `constrained` — so it is never a held Open and never enqueued as a loc.
    post_page(&iso, 2, &[(CONSTRAINED_ID, 1)]);
    tick(&iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "the constrained row is not a held Open"
    );
    let logs = iso.drain_logs();
    iso.join();
    assert_clean(&logs);
}

/// The finished collect over the `TaskBot` loop: the machine's own exact
/// `'clue solved'` status reaches the embed's `setStatus`, the `grind-ready`
/// handback and the `done` that follows keep the loop running with no verb,
/// the token dies with the end, and the sibling grind task takes the next
/// tick. The mark is the isolate machine's — this adapter never invents it.
#[test]
fn solve_clue_adapter_finishes_the_collect_with_the_solved_mark() {
    let src = r#"
import { SolveClue } from '../../api/ai/clues/SolveClue.js';
export default class T extends TaskBot {
    onStart() {
        globalThis.__statuses = [];
        globalThis.__logs = [];
        this.solveClue = new SolveClue({
            enabled: () => true,
            log: (message) => { globalThis.__logs.push(String(message)); },
            setStatus: (message) => { globalThis.__statuses.push(String(message)); },
        });
        this.add(this.solveClue, {
            validate: () => true,
            execute: async () => { globalThis.__grind = (globalThis.__grind || 0) + 1; },
        });
    }
}
"#;
    let iso = spawn(src);
    let casket = [(CASKET_ID, 1)];
    let quiet = Scene {
        main_modal_id: -1,
        ..Scene::default()
    };

    // Tick 1: the begin, the gate, the landed report and the casket's own
    // Open. The Open is the one step that yields, so it is the tick's verb.
    post_page(&iso, 1, &casket);
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Held {
            name: "Casket".to_string(),
            action: "Open".to_string(),
        }],
        "the casket Open is the landed held step"
    );

    // Tick 2: the casket left the pack. The reward interface is not posted, so
    // this call has nothing to close and nothing to take.
    post_scene(&iso, 2, &[], &quiet);
    tick(&iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "an empty reward page takes nothing"
    );

    // The frozen reward window is live time: past it the collect is over, and
    // the completion is the machine's own three steps — none of them a verb.
    std::thread::sleep(std::time::Duration::from_millis(2_100));
    post_scene(&iso, 3, &[], &quiet);
    tick(&iso, 3);
    assert!(
        iso.drain_interacts().is_empty(),
        "the completion queues no verb"
    );

    // Tick 4: nothing is held, so the finished clue task gives the tick back.
    post_scene(&iso, 4, &[], &quiet);
    tick(&iso, 4);

    let value = json(
        &iso,
        "JSON.stringify({ statuses: globalThis.__statuses, logs: globalThis.__logs, \
          grind: globalThis.__grind, token: globalThis.__rs_bot.solveClue.token, \
          status: globalThis.__rs_bot.solveClue.clueStatus() })",
    );
    let logs = iso.drain_logs();
    iso.join();
    assert_eq!(
        value["statuses"],
        serde_json::json!(["clue: trail_clue_hard_sextant028_casket", "clue solved"]),
        "the progress line and then the machine's own solved mark: {value:?}"
    );
    assert_eq!(
        value["status"], "clue solved",
        "the adapter remembers the machine's message, it does not invent it: {value:?}"
    );
    assert_eq!(
        value["token"],
        serde_json::Value::Null,
        "the `done` kills the token: {value:?}"
    );
    assert_eq!(
        value["grind"], 1,
        "the finished session hands the tick back: {value:?}"
    );
    assert!(
        !value.to_string().contains("ownsEquipment"),
        "the strip flag stays false and unpromoted: {value:?}"
    );
    assert_clean(&logs);
}

/// The posted death over the `TaskBot` loop: any live call whose posted
/// effective `hitpoints` is at or below zero ends the session — the adapter
/// clears the token, queues no verb and marks nothing solved — and the sibling
/// grind task takes the next tick. A page that posted no stat is not a death.
#[test]
fn solve_clue_adapter_ends_the_session_on_a_posted_death() {
    let src = r#"
import { SolveClue } from '../../api/ai/clues/SolveClue.js';
export default class T extends TaskBot {
    onStart() {
        globalThis.__statuses = [];
        globalThis.__logs = [];
        this.solveClue = new SolveClue({
            enabled: () => true,
            log: (message) => { globalThis.__logs.push(String(message)); },
            setStatus: (message) => { globalThis.__statuses.push(String(message)); },
        });
        this.add(this.solveClue, {
            validate: () => true,
            execute: async () => { globalThis.__grind = (globalThis.__grind || 0) + 1; },
        });
    }
}
"#;
    let iso = spawn(src);
    let page = [(SEARCH_ID, 1)];
    let stats = [script::isolate_fb::StatInput {
        index: 3,
        name: "hitpoints",
        xp: 0,
        base: 40,
        effective: 0,
    }];

    // Tick 1: the session opens and idles — no posted `here`, so no verb.
    post_page(&iso, 1, &page);
    tick(&iso, 1);
    assert!(
        iso.drain_interacts().is_empty(),
        "no posted `here`, no verb"
    );
    assert_ne!(
        probe_text(&iso, "String(globalThis.__rs_bot.solveClue.token)"),
        "null",
        "the session is live before the death"
    );

    // Tick 2: the posted effective hitpoints read zero. The machine's `dead`,
    // and the adapter clears the token on it.
    post_scene(
        &iso,
        2,
        &page,
        &Scene {
            stats: &stats,
            ..Scene::default()
        },
    );
    tick(&iso, 2);
    assert!(
        iso.drain_interacts().is_empty(),
        "a death pushes no interact"
    );
    assert_eq!(
        machine_next(&iso, false)["kind"],
        "aborted",
        "the token is dead, and the end state is not a continue kind"
    );
    assert_eq!(
        probe_text(&iso, "String(globalThis.__rs_bot.solveClue.token)"),
        "null",
        "the adapter cleared the dead token"
    );

    // Tick 3: the row is gone, so the finished session gives the tick back and
    // marks nothing solved.
    post_page(&iso, 3, &[]);
    tick(&iso, 3);
    let value = json(
        &iso,
        "JSON.stringify({ statuses: globalThis.__statuses, grind: globalThis.__grind })",
    );
    let logs = iso.drain_logs();
    iso.join();
    assert_eq!(value["grind"], 1, "{value:?}");
    assert!(
        !value.to_string().contains("clue solved"),
        "a death is never a solved mark: {value:?}"
    );
    assert_clean(&logs);
}

/// The arrived dig with no Spade over the `TaskBot` loop: the machine's named
/// `supplies-needed` wait-class keeps the token live, the adapter delays a
/// tick exactly like `wait`, fetches nothing and gives the grind task no tick.
/// The same live token Digs once the pack posts the item.
#[test]
fn solve_clue_adapter_keeps_the_token_live_over_a_missing_spade() {
    let src = r#"
import { SolveClue } from '../../api/ai/clues/SolveClue.js';
export default class T extends TaskBot {
    onStart() {
        globalThis.__statuses = [];
        globalThis.__logs = [];
        this.solveClue = new SolveClue({
            enabled: () => true,
            log: (message) => { globalThis.__logs.push(String(message)); },
            setStatus: (message) => { globalThis.__statuses.push(String(message)); },
        });
        this.add(this.solveClue, {
            validate: () => true,
            execute: async () => { globalThis.__grind = (globalThis.__grind || 0) + 1; },
        });
    }
}
"#;
    let iso = spawn(src);
    let page = [(UNGUARDED_ID, 1)];
    let with_spade = [(UNGUARDED_ID, 1), (SPADE_ITEM, 1)];
    let names = [(SPADE_ITEM, "Spade")];
    let arrived = TileInput {
        x: 3160,
        z: 3251,
        level: 0,
    };

    // Tick 1: arrived on the decoded tile with no Spade posted. The machine's
    // wait-class reaches the adapter, which keeps the token and queues nothing.
    post_scene(
        &iso,
        1,
        &page,
        &Scene {
            here: Some(arrived),
            trio: true,
            ..Scene::default()
        },
    );
    tick(&iso, 1);
    assert!(iso.drain_interacts().is_empty(), "no Spade, no Dig");
    let token = probe_text(&iso, "String(globalThis.__rs_bot.solveClue.token)");
    assert_ne!(token, "null", "the wait-class is a live session");

    // Tick 2: the pack posts the Spade: the same token Digs.
    post_scene(
        &iso,
        2,
        &with_spade,
        &Scene {
            here: Some(arrived),
            names: &names,
            trio: true,
            ..Scene::default()
        },
    );
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Held {
            name: "Spade".to_string(),
            action: "Dig".to_string(),
        }],
        "the posted Spade is the Dig"
    );
    assert_eq!(
        probe_text(&iso, "String(globalThis.__rs_bot.solveClue.token)"),
        token,
        "the wait-class never re-began the session"
    );
    let value = json(
        &iso,
        "JSON.stringify({ statuses: globalThis.__statuses, grind: globalThis.__grind })",
    );
    let logs = iso.drain_logs();
    iso.join();
    assert!(
        value["grind"].is_null(),
        "a live wait-class never hands the tick to the grind: {value:?}"
    );
    assert!(
        !value.to_string().contains("clue solved"),
        "a missing Spade is never a solved mark: {value:?}"
    );
    assert_clean(&logs);
}
/// `trail_clue_easy_simple005`: the talk membership whose jm2 spawn is unique —
/// `hans` at `(3207, 3233, 0)`.
const TALK_ID: i32 = 2681;
const TALK_X: i32 = 3207;
const TALK_Z: i32 = 3233;
/// `trail_clue_medium_anagram001`: the challenge parent `Hazelmere`, whose
/// `2842` scroll answers `"6859"`.
const TALK_CHALLENGE_ID: i32 = 2841;
const CHALLENGE_ID: i32 = 2842;
const CHALLENGE_ANSWER: i32 = 6859;

/// One posted npc row as SNAP posts it and the talk arm reads it.
fn scene_npc<'a>(
    index: i32,
    id: i32,
    name: &'a str,
    tile: TileInput,
    distance: i32,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index,
        id,
        name: Some(name),
        x: tile.x,
        z: tile.z,
        level: tile.level,
        distance,
        health: 0,
        max_health: 0,
        in_combat: false,
        animating: false,
        actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 1,
        nx: tile.x,
        nz: tile.z,
    }
}

/// The compat adapter's own talk drain: the machine's `walk` and `npc` kinds
/// reach the interact drain as `InteractReq::Walk` / `InteractReq::Npc`, an
/// open chat stops the Talk-to, and the empty page ends the session with no
/// solved mark and no second begin.
#[test]
fn solve_clue_adapter_drains_the_talk_step_over_the_task_loop() {
    let src = r#"
import { SolveClue } from '../../api/ai/clues/SolveClue.js';
export default class T extends TaskBot {
    onStart() {
        globalThis.__logs = [];
        globalThis.__statuses = [];
        this.solveClue = new SolveClue({
            enabled: () => true,
            log: (message) => { globalThis.__logs.push(String(message)); },
            setStatus: (message) => { globalThis.__statuses.push(String(message)); },
        });
        this.add(this.solveClue);
    }
}
"#;
    let iso = spawn(src);
    let page = [(TALK_ID, 1)];
    let talk_to = vec!["Talk-to".to_string()];
    let on_tile = [scene_npc(
        3,
        0,
        "Hans",
        TileInput {
            x: TALK_X,
            z: TALK_Z,
            level: 0,
        },
        1,
        &talk_to,
    )];

    // Tick 1: identified and reported. A page with no posted `here` is no
    // arrival claim, so no verb rides along.
    post_page(&iso, 1, &page);
    tick(&iso, 1);
    assert!(
        iso.drain_interacts().is_empty(),
        "a missing `here` is a wait, not a walk"
    );

    // Tick 2: posted far from the published tile — the machine's own walk.
    post_scene(
        &iso,
        2,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 3100,
                z: 3233,
                level: 0,
            }),
            ..Scene::default()
        },
    );
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![walk_to(TALK_X, TALK_Z, 0)],
        "the walk is the published x/z/plane"
    );

    // Tick 3: arrived with the step's own npc posted: the Talk-to.
    post_scene(
        &iso,
        3,
        &page,
        &Scene {
            here: Some(TileInput {
                x: TALK_X,
                z: TALK_Z,
                level: 0,
            }),
            npcs: &on_tile,
            ..Scene::default()
        },
    );
    tick(&iso, 3);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Hans".to_string(),
            action: "Talk-to".to_string(),
            index: Some(3),
        }],
        "the Talk-to keeps the posted identity and index"
    );

    // Tick 4: the same scene behind an open chat: no second Talk-to.
    post_scene(
        &iso,
        4,
        &page,
        &Scene {
            here: Some(TileInput {
                x: TALK_X,
                z: TALK_Z,
                level: 0,
            }),
            npcs: &on_tile,
            chat_modal_id: 968,
            ..Scene::default()
        },
    );
    tick(&iso, 4);
    assert!(
        iso.drain_interacts().is_empty(),
        "an open chat blocks the Talk-to"
    );

    // Tick 5: the posted `chat_continue` half of the same gate.
    post_scene(
        &iso,
        5,
        &page,
        &Scene {
            here: Some(TileInput {
                x: TALK_X,
                z: TALK_Z,
                level: 0,
            }),
            npcs: &on_tile,
            chat_continue: true,
            ..Scene::default()
        },
    );
    tick(&iso, 5);
    assert!(
        iso.drain_interacts().is_empty(),
        "a posted continue blocks the Talk-to too"
    );

    // Tick 6: a posted hitpoints at zero ends the session: the token is gone
    // and nothing was solved.
    let dead = [script::isolate_fb::StatInput {
        index: 3,
        name: "hitpoints",
        xp: 0,
        base: 40,
        effective: 0,
    }];
    post_scene(
        &iso,
        6,
        &page,
        &Scene {
            here: Some(TileInput {
                x: TALK_X,
                z: TALK_Z,
                level: 0,
            }),
            npcs: &on_tile,
            stats: &dead,
            ..Scene::default()
        },
    );
    tick(&iso, 6);
    assert!(
        iso.drain_interacts().is_empty(),
        "a death pushes no interact"
    );
    assert_eq!(
        probe_text(&iso, "String(globalThis.__rs_bot.solveClue.token)"),
        "null",
        "the dead token is released, not reused"
    );

    // Tick 7: the empty page is the landed C3 refusal: the session still has
    // no token, and nothing was solved.
    post_page(&iso, 7, &[]);
    tick(&iso, 7);
    assert!(
        iso.drain_interacts().is_empty(),
        "a refused begin has no verbs"
    );
    let value = json(
        &iso,
        "JSON.stringify({ token: String(globalThis.__rs_bot.solveClue.token), \
          status: globalThis.__rs_bot.solveClue.clueStatus(), \
          logs: globalThis.__logs, statuses: globalThis.__statuses })",
    );
    let logs = iso.drain_logs();
    iso.join();
    assert!(
        !value.to_string().contains("clue solved"),
        "no talk step is ever a solved mark: {value:?}"
    );
    assert_eq!(
        value["token"], "null",
        "the refused begin leaves no live token: {value:?}"
    );
    assert_clean(&logs);
}

/// The trio acquire chain over the compat adapter's own drain: the machine's
/// `walk`, `npc`, `answer` and `continue` steps reach the interact drain as
/// `InteractReq::Walk` / `Npc` / `Answer` / `ContinueDialog` — this adapter's
/// own arms, never the loc fall-through — and once the pack holds the trio the
/// same token falls through to the landed Dig.
#[test]
fn solve_clue_adapter_drains_the_trio_acquire_over_the_task_loop() {
    let src = r#"
import { SolveClue } from '../../api/ai/clues/SolveClue.js';
export default class T extends TaskBot {
    onStart() {
        globalThis.__logs = [];
        globalThis.__statuses = [];
        this.solveClue = new SolveClue({
            enabled: () => true,
            log: (message) => { globalThis.__logs.push(String(message)); },
            setStatus: (message) => { globalThis.__statuses.push(String(message)); },
        });
        this.add(this.solveClue);
    }
}
"#;
    let iso = spawn(src);
    let data = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let giver = data
        .trio_givers()
        .expect("trio_givers")
        .rows
        .iter()
        .find(|row| row.alias == "observatory_professor")
        .expect("the professor row")
        .clone();
    let spawn = giver.spawn.as_ref().expect("published spawn");
    let page = [(UNGUARDED_ID, 1)];
    let with_spade = [(UNGUARDED_ID, 1), (SPADE_ITEM, 1)];
    let names = [(SPADE_ITEM, "Spade")];
    let far = TileInput {
        x: 3100,
        z: 3300,
        level: 0,
    };
    let arrived = TileInput {
        x: 3160,
        z: 3251,
        level: 0,
    };
    let professor = TileInput {
        x: spawn.x,
        z: spawn.z,
        level: spawn.plane,
    };
    let talk_to = vec!["Talk-to".to_string()];
    let posted = [scene_npc(
        9,
        giver.id,
        giver.name.as_str(),
        professor,
        1,
        &talk_to,
    )];

    // Tick 1: no posted `here` at all, and no trio held: no arrival claim and
    // no blind walk.
    post_page(&iso, 1, &page);
    tick(&iso, 1);
    assert!(iso.drain_interacts().is_empty(), "no `here` is a wait");

    // Tick 2: still no trio: the chain's own walk is the giver's published
    // tile, never the row's decoded one.
    post_scene(
        &iso,
        2,
        &page,
        &Scene {
            here: Some(far),
            ..Scene::default()
        },
    );
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![walk_to(spawn.x, spawn.z, spawn.plane)],
        "the acquire chain walks to the published tile"
    );

    // Tick 3: arrived with the giver posted: the Talk-to.
    post_scene(
        &iso,
        3,
        &page,
        &Scene {
            here: Some(professor),
            npcs: &posted,
            ..Scene::default()
        },
    );
    tick(&iso, 3);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: giver.name.clone(),
            action: "Talk-to".to_string(),
            index: Some(9),
        }],
        "the Talk-to is the posted identity and index"
    );

    // Tick 4: the closed handler's option, posted second: that posted 1-based
    // slot is the answer, through the clue adapter's own arm.
    let options = [
        script::isolate_fb::ChatOptionInput {
            text: "Who are you?",
        },
        script::isolate_fb::ChatOptionInput {
            text: "Talk about Treasure Trails.",
        },
    ];
    post_scene(
        &iso,
        4,
        &page,
        &Scene {
            here: Some(professor),
            npcs: &posted,
            chat_modal_id: 968,
            options: &options,
            ..Scene::default()
        },
    );
    tick(&iso, 4);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Answer { option: 2 }],
        "the one selected literal is the answer, and it is never a loc"
    );

    // Tick 5: the linear half: the posted `chat_continue`.
    post_scene(
        &iso,
        5,
        &page,
        &Scene {
            here: Some(professor),
            npcs: &posted,
            chat_modal_id: 968,
            chat_continue: true,
            ..Scene::default()
        },
    );
    tick(&iso, 5);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::ContinueDialog],
        "the posted continue is the landed continue step"
    );

    // Tick 6: the posted pack holds the trio: the same token falls through to
    // the landed Dig on the row's own decoded tile.
    post_scene(
        &iso,
        6,
        &with_spade,
        &Scene {
            here: Some(arrived),
            names: &names,
            trio: true,
            ..Scene::default()
        },
    );
    tick(&iso, 6);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Held {
            name: "Spade".to_string(),
            action: "Dig".to_string(),
        }],
        "the held trio is the fall-through to the Dig"
    );

    let value = json(
        &iso,
        "JSON.stringify({ token: String(globalThis.__rs_bot.solveClue.token), \
          logs: globalThis.__logs, statuses: globalThis.__statuses })",
    );
    let logs = iso.drain_logs();
    iso.join();
    assert_ne!(
        value["token"], "null",
        "the chain never re-began: {value:?}"
    );
    let text = value.to_string();
    for forbidden in ["clue solved", "supplies-needed", "guardian-lost", "abandon"] {
        assert!(!text.contains(forbidden), "{forbidden} {value:?}");
    }
    assert_clean(&logs);
}

/// The compat adapter's challenge path: a page that holds only the selected
/// `2842` scroll opens a session on its parent talk step, and the posted count
/// dialog reaches the drain as `InteractReq::AnswerCount` — the clue adapter's
/// own arm, never the loc fall-through.
#[test]
fn solve_clue_adapter_answers_the_count_dialog_over_the_task_loop() {
    let src = r#"
import { SolveClue } from '../../api/ai/clues/SolveClue.js';
export default class T extends TaskBot {
    onStart() {
        globalThis.__logs = [];
        globalThis.__statuses = [];
        this.solveClue = new SolveClue({
            enabled: () => true,
            log: (message) => { globalThis.__logs.push(String(message)); },
            setStatus: (message) => { globalThis.__statuses.push(String(message)); },
        });
        this.add(this.solveClue);
    }
}
"#;
    let iso = spawn(src);
    let page = [(CHALLENGE_ID, 1)];
    let talk_to = vec!["Talk-to".to_string()];

    // Tick 1: the scroll alone, behind the posted count dialog. The seam joins
    // the parent, and the answer is the selected string.
    post_scene(
        &iso,
        1,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 2678,
                z: 3086,
                level: 1,
            }),
            count_dialog_open: true,
            ..Scene::default()
        },
    );
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::AnswerCount {
            value: CHALLENGE_ANSWER,
        }],
        "the count dialog takes the selected answer"
    );

    // Tick 2: the dialog is closed and the parent's npc is posted on the
    // published plane-1 tile: the parent's own Talk-to.
    let hazelmere = [scene_npc(
        7,
        669,
        "Hazelmere",
        TileInput {
            x: 2678,
            z: 3086,
            level: 1,
        },
        1,
        &talk_to,
    )];
    post_scene(
        &iso,
        2,
        &page,
        &Scene {
            here: Some(TileInput {
                x: 2678,
                z: 3086,
                level: 1,
            }),
            npcs: &hazelmere,
            ..Scene::default()
        },
    );
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Npc {
            name: "Hazelmere".to_string(),
            action: "Talk-to".to_string(),
            index: Some(7),
        }],
        "the parent's own Talk-to follows the answer"
    );

    let value = json(
        &iso,
        "JSON.stringify({ token: String(globalThis.__rs_bot.solveClue.token), \
          logs: globalThis.__logs, statuses: globalThis.__statuses })",
    );
    let logs = iso.drain_logs();
    iso.join();
    let text = value.to_string();
    assert!(
        text.contains("trail_clue_medium_anagram001"),
        "the step is the parent's: {value:?}"
    );
    assert!(
        text.contains(&TALK_CHALLENGE_ID.to_string()),
        "the parent id is the step: {value:?}"
    );
    assert!(
        !text.contains("2842"),
        "the scroll id is never the step: {value:?}"
    );
    assert!(!text.contains("clue solved"), "{value:?}");
    assert_clean(&logs);
}

/// The Entrana strip and its restore over the whole public path: the box row
/// `3579` is stripped — the frozen matcher's names unequipped with the landed
/// `wear` verb, the non-matches left worn, the hard-trail dagger id unequipped
/// and never listed — the listed name is deposited at the posted booth, the
/// `ownsEquipment()` seat reads the machine's own list from false to true and
/// back, `retry()` keeps the live token and the list while clearing the
/// abandon latch, and the collect's exit wears the name back on before the
/// exact `'clue solved'`, the `grind-ready` and the `done`.
#[test]
fn solve_clue_adapter_strips_and_restores_entrana_gear_before_the_solved_mark() {
    let src = r#"
import { SolveClue } from '../../api/ai/clues/SolveClue.js';
export default class T extends TaskBot {
    onStart() {
        globalThis.__logs = [];
        globalThis.__statuses = [];
        this.solveClue = new SolveClue({
            enabled: () => true,
            log: (message) => { globalThis.__logs.push(String(message)); },
            setStatus: (message) => { globalThis.__statuses.push(String(message)); },
        });
        this.add(this.solveClue);
    }
}
"#;
    let iso = spawn(src);
    let page = [(ENTRANA_ID, 1)];
    let booth_tile = TileInput {
        x: 2810,
        z: 3350,
        level: 0,
    };

    // Tick 1: the gate, and the strip's first unequip. The dagger is first in
    // posted order, and its id is never listed: the adapter still owns
    // nothing.
    let dagger_first = [
        worn_row(DDS_ITEM, "Dragon dagger(p)", 3),
        worn_row(HELM_ITEM, "Rune full helm", 0),
        worn_row(GLORY_ITEM, "Amulet of glory", 2),
    ];
    post_scene(
        &iso,
        1,
        &page,
        &Scene {
            equipment: &dagger_first,
            ..Scene::default()
        },
    );
    tick(&iso, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Wear {
            name: "Dragon dagger(p)".to_string(),
        }],
        "the first folded name is the landed unequip"
    );
    assert_eq!(
        probe_text(
            &iso,
            "String(globalThis.__rs_bot.solveClue.ownsEquipment())"
        ),
        "false",
        "a dagger id is never listed"
    );

    // Tick 2: the dagger gone from the worn page — the helm is next, and the
    // adapter's own read follows the machine's list. `retry()` in the middle of
    // the strip is the latch clear and nothing else.
    let helm_left = [
        worn_row(HELM_ITEM, "Rune full helm", 0),
        worn_row(GLORY_ITEM, "Amulet of glory", 2),
    ];
    post_scene(
        &iso,
        2,
        &page,
        &Scene {
            equipment: &helm_left,
            ..Scene::default()
        },
    );
    tick(&iso, 2);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Wear {
            name: "Rune full helm".to_string(),
        }],
        "the next folded name is unequipped"
    );
    let probe = json(
        &iso,
        "JSON.stringify({ owns: globalThis.__rs_bot.solveClue.ownsEquipment(), \
          retried: globalThis.__rs_bot.solveClue.retry(), \
          after: globalThis.__rs_bot.solveClue.ownsEquipment(), \
          token: String(globalThis.__rs_bot.solveClue.token), \
          validate: globalThis.__rs_bot.solveClue.validate() })",
    );
    assert_eq!(probe["owns"], true, "{probe:?}");
    assert_eq!(probe["retried"], true, "{probe:?}");
    assert_eq!(
        probe["after"], true,
        "retry never touches the list: {probe:?}"
    );
    assert_ne!(probe["token"], "null", "retry never aborts: {probe:?}");
    assert_eq!(probe["validate"], true, "{probe:?}");

    // Tick 3: the unequip landed in the pack, so the strip owes the bank trip
    // its walk. The worn page still carries the name the monks let through,
    // and the pack page is the identify page it always was.
    let held = [(ENTRANA_ID, 1), (HELM_ITEM, 1)];
    let names = [(HELM_ITEM, "Rune full helm")];
    let amulet_only = [worn_row(GLORY_ITEM, "Amulet of glory", 2)];
    post_scene(
        &iso,
        3,
        &held,
        &Scene {
            equipment: &amulet_only,
            names: &names,
            here: Some(booth_tile),
            nearest_booth: Some(&POSTED_BOOTH),
            ..Scene::default()
        },
    );
    tick(&iso, 3);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::WalkNearestBank],
        "the deposit's own stand walk, picked in Rust"
    );

    // Tick 4: beside the posted booth — its own identity, and never a copied
    // tile.
    post_scene(
        &iso,
        4,
        &held,
        &Scene {
            equipment: &amulet_only,
            names: &names,
            here: Some(booth_tile),
            nearest_booth: Some(&POSTED_BOOTH),
            ..Scene::default()
        },
    );
    tick(&iso, 4);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::OpenBooth {
            x: 2810,
            z: 3350,
            level: 0,
            id: 2213,
            name: Some("Bank booth".to_string()),
            action: Some("Use-quickly".to_string()),
        }],
        "the posted booth's own open, no invented stand"
    );

    // Tick 5: the interface is up — the listed name is deposited by name.
    post_scene(
        &iso,
        5,
        &held,
        &Scene {
            equipment: &amulet_only,
            names: &names,
            here: Some(booth_tile),
            nearest_booth: Some(&POSTED_BOOTH),
            bank_open: true,
            ..Scene::default()
        },
    );
    tick(&iso, 5);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Deposit {
            name: "Rune full helm".to_string(),
        }],
        "the regex-restricted name and nothing else is deposited"
    );

    // Tick 6: it landed — the interface closes.
    post_scene(
        &iso,
        6,
        &page,
        &Scene {
            equipment: &amulet_only,
            here: Some(booth_tile),
            nearest_booth: Some(&POSTED_BOOTH),
            bank_open: true,
            ..Scene::default()
        },
    );
    tick(&iso, 6);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Close],
        "the strip's own close before the row's arms run"
    );

    // Tick 7: the strip is settled and the row's own search arm runs: the walk
    // to its selected decode, with the name still listed for the restore.
    post_scene(
        &iso,
        7,
        &page,
        &Scene {
            equipment: &amulet_only,
            here: Some(booth_tile),
            nearest_booth: Some(&POSTED_BOOTH),
            ..Scene::default()
        },
    );
    tick(&iso, 7);
    assert_eq!(
        iso.drain_interacts(),
        vec![walk_to(ENTRANA_X, ENTRANA_Z, 0)],
        "the box row's own walk follows the settled strip"
    );
    assert_eq!(
        probe_text(
            &iso,
            "String(globalThis.__rs_bot.solveClue.ownsEquipment())"
        ),
        "true",
        "the listed name outlives the strip step"
    );

    // Tick 8: the machine's own abandon terminal — no production trigger, so
    // the seat is the latch's write — and the tick that lets the adapter see
    // the dead token.
    let left = json(
        &iso,
        "JSON.stringify(globalThis.rustyscript.functions.__rs2b0t_clue({ op: 'abandon' }))",
    );
    assert_eq!(left["kind"], "abandon", "{left:?}");
    post_scene(
        &iso,
        8,
        &page,
        &Scene {
            equipment: &amulet_only,
            ..Scene::default()
        },
    );
    tick(&iso, 8);
    assert!(
        iso.drain_interacts().is_empty(),
        "the terminal queues nothing"
    );
    assert_eq!(
        probe_text(&iso, "String(globalThis.__rs_bot.solveClue.token)"),
        "null",
        "the abandon terminal kills the token the adapter held"
    );
    // The same row, still held: `validate` is false and the begin is refused
    // with the latch's own token. `retry()` clears it and the same row begins
    // again.
    let refused = json(
        &iso,
        "JSON.stringify({ validate: globalThis.__rs_bot.solveClue.validate(), \
          begin: globalThis.rustyscript.functions.__rs2b0t_clue({ \
            op: 'begin', generation: 0, held: [[3579, 1]] }).reason, \
          retried: globalThis.__rs_bot.solveClue.retry(), \
          again: globalThis.__rs_bot.solveClue.validate() })",
    );
    assert_eq!(refused["validate"], false, "{refused:?}");
    assert_eq!(refused["begin"], "abandoned", "{refused:?}");
    assert_eq!(refused["retried"], true, "{refused:?}");
    assert_eq!(refused["again"], true, "{refused:?}");
    assert_eq!(
        probe_text(
            &iso,
            "String(globalThis.__rs_bot.solveClue.ownsEquipment())"
        ),
        "true",
        "the latch clear is not the list's: {refused:?}"
    );

    // Tick 9: the casket is held instead, so the trail reaches its collect.
    // The casket is never a strip row.
    let casket = [(CASKET_ID, 1)];
    post_page(&iso, 9, &casket);
    tick(&iso, 9);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Held {
            name: "Casket".to_string(),
            action: "Open".to_string(),
        }],
        "the held casket's own Open"
    );

    // Tick 10: the casket left. The reward window is real time, so the collect
    // waits it out before its exit.
    let quiet = Scene {
        equipment: &amulet_only,
        main_modal_id: -1,
        ..Scene::default()
    };
    post_scene(&iso, 10, &[], &quiet);
    tick(&iso, 10);
    assert!(
        iso.drain_interacts().is_empty(),
        "an empty reward page takes nothing"
    );
    std::thread::sleep(std::time::Duration::from_millis(2_100));

    // Tick 11: the collect is over and the name is in the pack: the reclaim
    // wears it back on before any completion kind, and no `'clue solved'` has
    // gone out with the name still listed.
    let back = [(HELM_ITEM, 1)];
    let back_names = [(HELM_ITEM, "Rune full helm")];
    post_scene(
        &iso,
        11,
        &back,
        &Scene {
            names: &back_names,
            equipment: &amulet_only,
            main_modal_id: -1,
            ..Scene::default()
        },
    );
    tick(&iso, 11);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::Wear {
            name: "Rune full helm".to_string(),
        }],
        "the collect's exit wears the listed name back on"
    );
    assert_eq!(
        probe_text(
            &iso,
            "String((globalThis.__statuses || []).indexOf('clue solved') === -1)"
        ),
        "true",
        "no solved mark rides the reclaim"
    );
    assert_eq!(
        probe_text(
            &iso,
            "String(globalThis.__rs_bot.solveClue.ownsEquipment())"
        ),
        "true",
        "the name is listed until it is worn again"
    );

    // Tick 12: worn again. The list empties, and the exact status, the
    // `grind-ready` and the `done` follow — and the adapter's own read is
    // finally false.
    post_scene(
        &iso,
        12,
        &[],
        &Scene {
            equipment: &[worn_row(HELM_ITEM, "Rune full helm", 0)],
            main_modal_id: -1,
            ..Scene::default()
        },
    );
    tick(&iso, 12);
    assert!(
        iso.drain_interacts().is_empty(),
        "the completion queues no verb"
    );
    let value = json(
        &iso,
        "JSON.stringify({ statuses: globalThis.__statuses, logs: globalThis.__logs, \
          token: String(globalThis.__rs_bot.solveClue.token), \
          owns: globalThis.__rs_bot.solveClue.ownsEquipment(), \
          status: globalThis.__rs_bot.solveClue.clueStatus() })",
    );
    let logs = iso.drain_logs();
    iso.join();
    let statuses = value["statuses"]
        .as_array()
        .expect("status lines")
        .iter()
        .filter_map(|line| line.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        statuses.last().copied(),
        Some("clue solved"),
        "the machine's own mark is the last status: {value:?}"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|line| line.contains("clue solved"))
            .count(),
        1,
        "exactly one solved mark, and only after the reclaim: {value:?}"
    );
    assert!(
        statuses
            .iter()
            .any(|line| line.contains("trail_clue_hard_riddle027")),
        "the stripped row's own identity was reported: {value:?}"
    );
    assert_eq!(value["status"], "clue solved", "{value:?}");
    assert_eq!(
        value["token"], "null",
        "the done kills the token: {value:?}"
    );
    assert_eq!(
        value["owns"], false,
        "the machine's list is empty and the adapter reads it: {value:?}"
    );
    let text = value.to_string();
    for forbidden in [
        "restore-incomplete",
        "restore-walk-failed",
        "supplies-needed",
    ] {
        assert!(
            !text.contains(forbidden),
            "the happy path logs no failure: {forbidden} {value:?}"
        );
    }
    assert_clean(&logs);
}
