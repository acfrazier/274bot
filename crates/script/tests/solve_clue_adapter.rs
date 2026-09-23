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
/// The packed `access: "constrained"` row: identified, then idle.
const CONSTRAINED_ID: i32 = 3554;
/// The casket the constrained row's own selected `trail_casket` names.
const CASKET_ID: i32 = 3555;
/// The decoded tile `SEARCH_ID` walks to.
const SEARCH_X: i32 = 3209;
const SEARCH_Z: i32 = 3218;
const SEARCH_LEVEL: i32 = 1;

/// The posted pages these tests need: the pack page is the `page` argument,
/// plus this call's `here` tile and loc page and the posted `hold || ours`
/// pair.
struct Scene<'a> {
    here: Option<TileInput>,
    locs: &'a [SceneEntityInput<'a>],
    hold: bool,
    ours: bool,
}

impl Default for Scene<'_> {
    fn default() -> Self {
        Self {
            here: None,
            locs: &[],
            hold: false,
            ours: false,
        }
    }
}

fn post_page(iso: &LoadIsolate, tick: u64, page: &[(i32, i32)]) {
    post_scene(iso, tick, page, &Scene::default());
}

fn post_scene(iso: &LoadIsolate, tick: u64, page: &[(i32, i32)], scene: &Scene<'_>) {
    let rows: Vec<ItemRowInput<'_>> = page
        .iter()
        .enumerate()
        .map(|(slot, (id, count))| ItemRowInput {
            name: None,
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
        withdraw_load_result_seq: 0,
        withdraw_load_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
        hold: scene.hold,
        ours: scene.ours,
        npcs: &[],
        locs: scene.locs,
        players: &[],
        ground: &[],
        equipment: &[],
        chat_open: false,
        chat_continue: false,
        chat_text: None,
        chat_options: &[],
        side_tab: -1,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: None,
        in_combat: false,
        animating: false,
        main_modal_id: 0,
        chat_modal_id: -1,
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
    assert_eq!(value["retry"], "undefined", "{value:?}");
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
    assert_eq!(value["statuses"].as_array().map(Vec::len), Some(1), "{value:?}");
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
    assert!(queued.is_empty(), "a disabled tick queues nothing: {queued:?}");
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
    assert!(iso.drain_interacts().is_empty(), "nothing to do, nothing queued");
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

    // The casket gone: the constrained clue is identified and idled, never
    // enqueued as a loc.
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
