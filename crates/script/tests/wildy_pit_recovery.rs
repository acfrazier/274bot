//! Live wolf-pit: ridge Open, fail chat, WalkNear from the ridge NoPath,
//! then the player stands in the pit. Frozen `walkResilient` keeps walking
//! until arrived or timeout; a one-shot WalkNear returns and EnterCourse
//! no longer validates (`southOfRidge` is false in the pit).

use script::isolate_fb::{
    encode_snapshot, encode_snapshot_with_native, ChatLineInput, NativeFactsInput, ReachViewInput,
    SceneEntityInput, SnapshotInput, TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

mod common;
use common::FRESH_STATS;

const CARD: &str = r#"
import { TaskBot } from '../../api/bot/Bot.js';
import { Execution } from '../../api/execution/Execution.js';
import { Game } from '../../api/game/Game.js';
import { GameMessages } from '../../api/chatbox/gameMessages.js';
import { Skills } from '../../api/skills/Skills.js';
import { Locs } from '../../api/locs/Locs.js';
import { Traversal } from '../../api/walking/Traversal.js';
import { EventSignal } from '../../api/execution/EventSignal.js';

const RIDGE_APPROACH = { x: 2998, z: 3916, level: 0 };
const RIDGE_DOOR = { x: 2998, z: 3917, level: 0 };
const RIDGE_FAIL = /you lose your footing and fall into the wolf pit/i;
const RIDGE_SUCCESS = /you skillfully balance across the ridge/i;

function southOfRidge(here) {
    return here.level === 0 && here.z <= RIDGE_DOOR.z;
}
function nearCourseEntry(here) {
    if (here.z > 3931 || here.z < RIDGE_APPROACH.z - 2) return false;
    return Math.abs(here.x - 2998) <= 10;
}
function atRidgeApproach(here, radius) {
    const d = Math.max(Math.abs(here.x - RIDGE_APPROACH.x), Math.abs(here.z - RIDGE_APPROACH.z));
    return d <= radius && southOfRidge(here);
}

async function walkToRidgeApproach(bot, attempts, timeoutMs) {
    const here = Game.tile();
    if (here && atRidgeApproach(here, 1)) return;
    await Traversal.walkResilient(RIDGE_APPROACH, {
        radius: 1,
        attempts,
        timeoutMs,
        log: (m) => bot.log('  ' + m),
    });
}

async function attemptRidgeCrossing(bot) {
    await walkToRidgeApproach(bot, 4, 60_000);
    const ridge = Locs.query().name('Door').action('Open').nearest();
    if (!ridge) {
        await Execution.delayTicks(2);
        return 'timeout';
    }
    const mark = GameMessages.mark();
    const beforeXp = Skills.xp('agility');
    if (!(await ridge.interact('Open'))) {
        await Execution.delayTicks(2);
        return 'timeout';
    }
    await Execution.delayUntil(() => {
        if (EventSignal.pending()) return true;
        if (Skills.xp('agility') > beforeXp) return true;
        if (GameMessages.sawSince(mark, RIDGE_SUCCESS)) return true;
        if (GameMessages.sawSince(mark, RIDGE_FAIL)) return true;
        return false;
    }, 10_000);
    const sawFail = GameMessages.sawSince(mark, RIDGE_FAIL);
    if (sawFail) {
        bot.log('fell into the wolf pit — walking back to ridge approach');
        await walkToRidgeApproach(bot, 3, 30_000);
        return 'fail';
    }
    return 'timeout';
}

class EnterCourse {
    constructor(bot) { this.bot = bot; }
    validate() {
        const here = Game.tile();
        if (!here) return false;
        if (here.z > 3931) return false;
        if (!nearCourseEntry(here) && !atRidgeApproach(here, 4)) return false;
        return southOfRidge(here);
    }
    async execute() {
        await attemptRidgeCrossing(this.bot);
    }
}

export default class WildyPit extends TaskBot {
    async onStart() {
        this.add(new EnterCourse(this));
    }
}
"#;

fn approach() -> TileInput {
    TileInput {
        x: 2998,
        z: 3916,
        level: 0,
    }
}

fn ridge() -> TileInput {
    TileInput {
        x: 2998,
        z: 3924,
        level: 0,
    }
}

fn pit() -> TileInput {
    TileInput {
        x: 3001,
        z: 3923,
        level: 0,
    }
}

fn loc_row<'a>(
    id: i32,
    name: Option<&'a str>,
    x: i32,
    z: i32,
    actions: &'a [String],
) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: id,
        id,
        name,
        x,
        z,
        level: 0,
        distance: 1,
        health: -1,
        max_health: -1,
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

fn snap<'a>(
    tick: u64,
    here: TileInput,
    locs: &'a [SceneEntityInput<'a>],
    chat: &'a [ChatLineInput<'a>],
    stats: &'a [script::isolate_fb::StatInput<'a>],
) -> SnapshotInput<'a> {
    SnapshotInput {
        tick,
        here: Some(here),
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats,
        booths: &[],
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
        hold: false,
        ours: false,
        npcs: &[],
        locs,
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
        my_name: Some("bot"),
        in_combat: false,
        animating: false,
        main_modal_id: -1,
        chat_modal_id: -1,
        make_products: &[],
        side_tab_ifaces: &[],
        spell_buttons: &[],
        chat_lines: chat,
        nearest_booth: None,
        bank_note_on: -1,
        bank_note_off: -1,
        scene_state: 2,
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
        reach: ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
    }
}

fn fail_native(seq: u64, request_id: u64) -> NativeFactsInput<'static> {
    NativeFactsInput {
        walk_outcome_seq: seq,
        walk_outcome_generation: 1,
        walk_outcome_request_id: request_id,
        walk_outcome_failed: true,
        walk_outcome_x: 2998,
        walk_outcome_z: 3916,
        walk_outcome_level: 0,
        walk_outcome_radius: 1,
        walk_outcome_allow_teleports: false,
        ..Default::default()
    }
}

fn is_approach_walk(req: &InteractReq) -> bool {
    matches!(
        req,
        InteractReq::WalkNear {
            x: 2998,
            z: 3916,
            level: 0,
            radius: 1,
            ..
        }
    )
}

fn walk_request_id(req: &InteractReq) -> u64 {
    match req {
        InteractReq::WalkNear { request_id, .. } => *request_id,
        _ => 0,
    }
}

/// Frozen `attemptRidgeCrossing` recovery: after the wolf-pit line, keep
/// walking to the south stand until arrived or the 30s bound. A NoPath
/// from the ridge tile must not end the recovery — the next tick in the
/// pit must issue another WalkNear.
#[test]
fn wolf_pit_nopath_retries_walk_from_the_pit() {
    let iso = LoadIsolate::spawn(CARD.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let open = ["Open".to_string()];
    let door = [loc_row(2309, Some("Door"), 2998, 3917, &open)];
    let stats = FRESH_STATS;

    iso.post_snapshot(encode_snapshot(&snap(1, approach(), &door, &[], &stats)));
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let first = iso.drain_interacts();
    assert!(
        first.iter().any(|req| matches!(
            req,
            InteractReq::Loc {
                x: 2998,
                z: 3917,
                action,
                id: Some(2309),
                ..
            } if action == "Open"
        )),
        "EnterCourse must Open the ridge door, got {first:?}"
    );

    let fall = [ChatLineInput {
        seq: 1,
        text: "You lose your footing and fall into the wolf pit.",
        type_: 0,
        username: None,
    }];
    iso.post_snapshot(encode_snapshot(&snap(2, ridge(), &door, &fall, &stats)));
    iso.on_game_tick(2);
    let _ = iso.probe("true");
    let recovery = iso.drain_interacts();
    let first_walk = recovery
        .iter()
        .find(|req| is_approach_walk(req))
        .expect("recovery WalkNear to the south stand");
    let request_id = walk_request_id(first_walk);
    assert_ne!(request_id, 0);

    iso.post_snapshot(encode_snapshot_with_native(
        &snap(3, pit(), &door, &fall, &stats),
        fail_native(1, request_id),
    ));
    iso.on_game_tick(3);
    let _ = iso.probe("true");
    let after_fail = iso.drain_interacts();
    iso.on_game_tick(4);
    let _ = iso.probe("true");
    let after_tick = iso.drain_interacts();

    let retried = after_fail
        .iter()
        .chain(after_tick.iter())
        .any(is_approach_walk);
    assert!(
        retried,
        "frozen walkResilient keeps walking after NoPath once the player is in the pit; \
         got after_fail={after_fail:?} after_tick={after_tick:?}"
    );
    iso.join();
}
