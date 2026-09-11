use script::isolate_fb::{ChatLineInput, ReachViewInput, SnapshotInput, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
}

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 3235,
            z: 3295,
            level: 0,
        }),
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats: &[],
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
        locs: &[],
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
        chat_lines: &[],
        nearest_booth: None,
        bank_note_on: -1,
        bank_note_off: -1,
        scene_state: 2,
        weight: 0,
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
        widgets: &[],
    }
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("true");
}

fn number(iso: &LoadIsolate, expr: &str) -> i64 {
    iso.probe(expr).ok().and_then(|v| v.as_i64()).unwrap_or(0)
}

fn lumbridge() -> TileInput {
    TileInput {
        x: 3222,
        z: 3218,
        level: 0,
    }
}

fn anchor() -> TileInput {
    TileInput {
        x: 3235,
        z: 3295,
        level: 0,
    }
}

fn welcome(seq: i32) -> ChatLineInput<'static> {
    ChatLineInput {
        seq,
        text: "Welcome to RuneScape",
    }
}

fn death(seq: i32) -> ChatLineInput<'static> {
    ChatLineInput {
        seq,
        text: "Oh dear, you are dead!",
    }
}

fn walk_near_anchor() -> InteractReq {
    InteractReq::WalkNear {
        x: 3235,
        z: 3295,
        level: 0,
        radius: 3,
        allow_teleports: false,
    }
}

const CHICKEN: &str = r#"
import { DeathRecovery } from '../../api/tasks/DeathRecovery.js';
export default class ChickenShaped extends TaskBot {
    onStart() {
        this.add(new DeathRecovery(this, {
            anchor: { x: 3235, z: 3295, level: 0 },
            radius: 3,
            onDeath: () => { globalThis.__deaths = (globalThis.__deaths || 0) + 1; },
            onRecovered: () => { globalThis.__recovered = (globalThis.__recovered || 0) + 1; },
        }));
    }
}
"#;

#[test]
fn idle_without_death_sends_nothing() {
    let iso = LoadIsolate::spawn(CHICKEN.into(), LoadShape::CompatClass, vec![]).unwrap();
    let lines = [welcome(1)];
    let mut snap = base_snapshot();
    snap.chat_lines = &lines;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    tick(&iso, 2);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(number(&iso, "globalThis.__deaths || 0"), 0);
    iso.join();
}

#[test]
fn seeded_duplicate_and_old_chat_do_not_retrigger() {
    let iso = LoadIsolate::spawn(CHICKEN.into(), LoadShape::CompatClass, vec![]).unwrap();
    let seeded = [death(4), welcome(3)];
    let mut snap = base_snapshot();
    snap.here = Some(lumbridge());
    snap.chat_lines = &seeded;
    snap.chat_text = Some("Oh dear, you are dead!");
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    tick(&iso, 2);
    tick(&iso, 3);
    assert_eq!(number(&iso, "globalThis.__deaths || 0"), 0);
    assert!(
        iso.drain_interacts().is_empty(),
        "seeded death chat must not start recovery"
    );

    post_snapshot_input(&iso, &snap);
    tick(&iso, 4);
    assert_eq!(number(&iso, "globalThis.__deaths || 0"), 0);

    let old = [welcome(3)];
    snap.chat_lines = &old;
    snap.tick = 5;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 5);
    assert_eq!(number(&iso, "globalThis.__deaths || 0"), 0);
    iso.join();
}

#[test]
fn new_death_waits_then_walks_and_recovers_once_from_actual_position() {
    let iso = LoadIsolate::spawn(CHICKEN.into(), LoadShape::CompatClass, vec![]).unwrap();
    let welcome_lines = [welcome(1)];
    let mut snap = base_snapshot();
    snap.chat_lines = &welcome_lines;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);

    let death_lines = [death(2), welcome(1)];
    snap.tick = 2;
    snap.here = Some(lumbridge());
    snap.chat_lines = &death_lines;
    snap.chat_text = Some("Oh dear, you are dead!");
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(number(&iso, "globalThis.__deaths || 0"), 1);
    assert!(
        iso.drain_interacts().is_empty(),
        "respawn wait must run before walk"
    );

    for n in 3..=6 {
        snap.tick = n;
        post_snapshot_input(&iso, &snap);
        tick(&iso, n);
    }
    assert_eq!(
        iso.drain_interacts(),
        vec![walk_near_anchor()],
        "queued walk is not recovery"
    );
    assert_eq!(number(&iso, "globalThis.__recovered || 0"), 0);

    snap.tick = 7;
    snap.here = Some(anchor());
    post_snapshot_input(&iso, &snap);
    tick(&iso, 7);
    assert!(iso.drain_interacts().is_empty());
    assert_eq!(number(&iso, "globalThis.__recovered || 0"), 1);

    snap.tick = 8;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 8);
    tick(&iso, 9);
    assert_eq!(number(&iso, "globalThis.__deaths || 0"), 1);
    assert_eq!(number(&iso, "globalThis.__recovered || 0"), 1);
    iso.join();
}

#[test]
fn wildy_walk_back_is_invoked_instead_of_default_walk() {
    let src = r#"
import { DeathRecovery } from '../../api/tasks/DeathRecovery.js';
export default class WildyShaped extends TaskBot {
    onStart() {
        this.add(new DeathRecovery(this, {
            anchor: { x: 3004, z: 3931, level: 0 },
            radius: 6,
            onDeath: () => { globalThis.__deaths = (globalThis.__deaths || 0) + 1; },
            onRecovered: () => { globalThis.__recovered = (globalThis.__recovered || 0) + 1; },
            walkBack: async () => { globalThis.__walkBack = (globalThis.__walkBack || 0) + 1; },
        }));
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let welcome_lines = [welcome(1)];
    let mut snap = base_snapshot();
    snap.here = Some(lumbridge());
    snap.chat_lines = &welcome_lines;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);

    let death_lines = [death(2), welcome(1)];
    snap.tick = 2;
    snap.chat_lines = &death_lines;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(number(&iso, "globalThis.__deaths || 0"), 1);

    for n in 3..=6 {
        snap.tick = n;
        post_snapshot_input(&iso, &snap);
        tick(&iso, n);
    }
    assert!(
        iso.drain_interacts()
            .iter()
            .all(|req| !matches!(req, InteractReq::WalkNear { .. })),
        "WildyAgility walkBack must retain the script-owned sequence"
    );
    assert_eq!(number(&iso, "globalThis.__walkBack || 0"), 1);
    assert_eq!(number(&iso, "globalThis.__recovered || 0"), 1);
    iso.join();
}

#[test]
fn needs_is_explicit_unsupported() {
    let src = r#"
import { DeathRecovery } from '../../api/tasks/DeathRecovery.js';
export default class T extends TaskBot {
    onStart() {
        this.add(new DeathRecovery(this, {
            anchor: { x: 3235, z: 3295, level: 0 },
            needs: [{ name: 'Coins', qty: 1 }],
        }));
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let snap = base_snapshot();
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);
    let logs = iso.drain_logs();
    assert!(
        logs.iter()
            .any(|line| line.contains("not impl: DeathRecovery.needs")),
        "needs/AcquireTask must stay an explicit unsupported boundary: {logs:?}"
    );
    iso.join();
}

#[test]
fn pause_hold_and_session_reset_drop_stale_walk() {
    let iso = LoadIsolate::spawn(CHICKEN.into(), LoadShape::CompatClass, vec![]).unwrap();
    let welcome_lines = [welcome(1)];
    let mut snap = base_snapshot();
    snap.chat_lines = &welcome_lines;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);

    let death_lines = [death(2), welcome(1)];
    snap.tick = 2;
    snap.here = Some(lumbridge());
    snap.chat_lines = &death_lines;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);
    assert_eq!(number(&iso, "globalThis.__deaths || 0"), 1);

    snap.tick = 3;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 3);
    iso.pause();
    snap.tick = 4;
    snap.here = Some(anchor());
    post_snapshot_input(&iso, &snap);
    tick(&iso, 4);
    assert!(
        iso.drain_interacts().is_empty(),
        "Pause must freeze deadlines and not send walk"
    );
    assert_eq!(number(&iso, "globalThis.__recovered || 0"), 0);

    iso.resume();
    iso.reset_session_work();
    snap.tick = 5;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 5);
    tick(&iso, 6);
    tick(&iso, 7);
    assert_eq!(number(&iso, "globalThis.__recovered || 0"), 0);
    assert!(
        iso.drain_interacts()
            .iter()
            .all(|req| !matches!(req, InteractReq::WalkNear { .. })),
        "Stop/session replacement must discard stale recovery"
    );
    iso.join();
}

#[test]
fn guardian_hold_does_not_walk() {
    let iso = LoadIsolate::spawn(CHICKEN.into(), LoadShape::CompatClass, vec![]).unwrap();
    let welcome_lines = [welcome(1)];
    let mut snap = base_snapshot();
    snap.chat_lines = &welcome_lines;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 1);

    let death_lines = [death(2), welcome(1)];
    snap.tick = 2;
    snap.here = Some(lumbridge());
    snap.chat_lines = &death_lines;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 2);

    snap.tick = 3;
    snap.hold = true;
    post_snapshot_input(&iso, &snap);
    tick(&iso, 3);
    tick(&iso, 4);
    tick(&iso, 5);
    tick(&iso, 6);
    assert!(
        iso.drain_interacts().is_empty(),
        "Guardian hold must freeze sends"
    );
    assert_eq!(number(&iso, "globalThis.__recovered || 0"), 0);
    iso.join();
}
