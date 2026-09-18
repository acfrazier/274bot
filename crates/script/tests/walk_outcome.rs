//! Native walk wait: LoadIsolate + FlatBuffer outcome, not arrival-only timeout.

use script::isolate_fb::{
    encode_snapshot, encode_snapshot_with_native, NativeFactsInput, ReachViewInput, SnapshotInput,
    TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

fn base_snapshot<'a>(tick: u64, here: TileInput) -> SnapshotInput<'a> {
    SnapshotInput {
        tick,
        here: Some(here),
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
        my_name: None,
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
        scene_state: 0,
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

fn far() -> TileInput {
    TileInput {
        x: 2823,
        z: 3555,
        level: 0,
    }
}

fn near() -> TileInput {
    TileInput {
        x: 2821,
        z: 3556,
        level: 0,
    }
}

fn fail_native(
    seq: u64,
    generation: u64,
    request_id: u64,
    x: i32,
    z: i32,
    radius: i32,
) -> NativeFactsInput<'static> {
    NativeFactsInput {
        walk_outcome_seq: seq,
        walk_outcome_generation: generation,
        walk_outcome_request_id: request_id,
        walk_outcome_failed: true,
        walk_outcome_x: x,
        walk_outcome_z: z,
        walk_outcome_level: 0,
        walk_outcome_radius: radius,
        walk_outcome_allow_teleports: false,
        ..Default::default()
    }
}

fn walk_src(timeout_ms: i32) -> String {
    format!(
        r#"
import {{ Traversal }} from '../../api/walking/Traversal.js';
export default class T extends LoopingBot {{
    async loop() {{
        globalThis.__rs_ok = null;
        globalThis.__rs_ok = await Traversal.walkResilient(
            {{ x: 2820, z: 3556, level: 0 }},
            {{ radius: 1, timeoutMs: {timeout_ms} }},
        );
    }}
}}
"#
    )
}

fn park_walk(iso: &LoadIsolate) -> u64 {
    // Snapshot, then tick: begin/queue run in the isolate. Drain proves the
    // WalkNear left on the producer wire before any outcome snapshot is posted.
    iso.post_snapshot(encode_snapshot(&base_snapshot(1, far())));
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__rs_ok").unwrap(), serde_json::Value::Null);
    let drained = iso.drain_interacts();
    let request_id = match &drained[..] {
        [InteractReq::WalkNear {
            x: 2820,
            z: 3556,
            level: 0,
            radius: 1,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id,
        }] => *request_id,
        other => panic!("unexpected interacts: {other:?}"),
    };
    assert_ne!(request_id, 0, "isolate must allocate a walk request id");
    request_id
}

#[test]
fn isolate_nopath_outcome_returns_false_promptly() {
    let iso = LoadIsolate::spawn(walk_src(300_000), LoadShape::CompatClass, vec![]).unwrap();
    let request_id = park_walk(&iso);
    iso.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(2, far()),
        fail_native(1, 0, request_id, 2820, 3556, 1),
    ));
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__rs_ok").unwrap(), false);
    iso.join();
}

#[test]
fn isolate_arrival_within_radius_returns_true() {
    let iso = LoadIsolate::spawn(walk_src(300_000), LoadShape::CompatClass, vec![]).unwrap();
    park_walk(&iso);
    iso.post_snapshot(encode_snapshot(&base_snapshot(2, near())));
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__rs_ok").unwrap(), true);
    iso.join();
}

#[test]
fn isolate_pending_keeps_the_caller_timeout() {
    let iso = LoadIsolate::spawn(walk_src(300_000), LoadShape::CompatClass, vec![]).unwrap();
    park_walk(&iso);
    iso.post_snapshot(encode_snapshot(&base_snapshot(2, far())));
    iso.on_game_tick(2);
    assert_eq!(
        iso.probe("__rs_ok").unwrap(),
        serde_json::Value::Null,
        "pending walk must not settle without arrival or matching NoPath"
    );
    iso.join();
}

#[test]
fn isolate_other_request_nopath_cannot_settle() {
    let iso = LoadIsolate::spawn(walk_src(300_000), LoadShape::CompatClass, vec![]).unwrap();
    let request_id = park_walk(&iso);
    iso.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(2, far()),
        fail_native(1, 0, request_id, 3200, 3200, 1),
    ));
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__rs_ok").unwrap(), serde_json::Value::Null);
    iso.join();
}

#[test]
fn isolate_stale_request_id_cannot_settle_a_new_wait() {
    let iso = LoadIsolate::spawn(walk_src(300_000), LoadShape::CompatClass, vec![]).unwrap();
    iso.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(1, far()),
        fail_native(4, 4, 4, 2820, 3556, 1),
    ));
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__rs_ok").unwrap(), serde_json::Value::Null);
    iso.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(2, far()),
        fail_native(4, 4, 4, 2820, 3556, 1),
    ));
    iso.on_game_tick(2);
    assert_eq!(
        iso.probe("__rs_ok").unwrap(),
        serde_json::Value::Null,
        "already-observed fail request id must not settle the later wait"
    );
    iso.join();
}

#[test]
fn isolate_pending_timeout_still_returns_false() {
    let iso = LoadIsolate::spawn(walk_src(100), LoadShape::CompatClass, vec![]).unwrap();
    park_walk(&iso);
    std::thread::sleep(std::time::Duration::from_millis(140));
    iso.post_snapshot(encode_snapshot(&base_snapshot(2, far())));
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__rs_ok").unwrap(), false);
    iso.join();
}

#[test]
fn isolate_drain_then_wrong_request_id_does_not_settle() {
    let iso = LoadIsolate::spawn(walk_src(300_000), LoadShape::CompatClass, vec![]).unwrap();
    let request_id = park_walk(&iso);
    iso.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(2, far()),
        fail_native(1, 0, request_id.wrapping_add(7), 2820, 3556, 1),
    ));
    iso.on_game_tick(2);
    assert_eq!(
        iso.probe("__rs_ok").unwrap(),
        serde_json::Value::Null,
        "same-target NoPath for a different request id must not settle"
    );
    iso.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(3, far()),
        fail_native(2, 0, request_id, 2820, 3556, 1),
    ));
    iso.on_game_tick(3);
    assert_eq!(iso.probe("__rs_ok").unwrap(), false);
    iso.join();
}

#[test]
fn isolate_same_target_old_request_id_does_not_settle_new_wait() {
    let iso = LoadIsolate::spawn(walk_src(300_000), LoadShape::CompatClass, vec![]).unwrap();
    iso.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(1, far()),
        fail_native(1, 5, 5, 2820, 3556, 1),
    ));
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__rs_ok").unwrap(), serde_json::Value::Null);
    let request_id = match &iso.drain_interacts()[..] {
        [InteractReq::WalkNear { request_id, .. }] => *request_id,
        other => panic!("unexpected interacts: {other:?}"),
    };
    iso.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(2, far()),
        fail_native(2, 5, 5, 2820, 3556, 1),
    ));
    iso.on_game_tick(2);
    assert_eq!(
        iso.probe("__rs_ok").unwrap(),
        serde_json::Value::Null,
        "late same-target NoPath for request id 5 must not settle the new wait"
    );
    iso.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(3, far()),
        fail_native(3, 6, request_id, 2820, 3556, 1),
    ));
    iso.on_game_tick(3);
    assert_eq!(iso.probe("__rs_ok").unwrap(), false);
    iso.join();
}

fn overlapping_walk_src() -> String {
    format!(
        r#"
import {{ Traversal }} from '../../api/walking/Traversal.js';
export default class T extends LoopingBot {{
    async loop() {{
        if (globalThis.__rs_done) return;
        globalThis.__rs_a = null;
        globalThis.__rs_b = null;
        Traversal.walkResilient(
            {{ x: 2820, z: 3556, level: 0 }},
            {{ radius: 1, timeoutMs: 300000 }},
        ).then(v => {{ globalThis.__rs_a = v; }});
        globalThis.__rs_b = await Traversal.walkResilient(
            {{ x: 2820, z: 3556, level: 0 }},
            {{ radius: 1, timeoutMs: 300000 }},
        );
        globalThis.__rs_done = true;
    }}
}}
"#
    )
}

fn park_two_walks(iso: &LoadIsolate) -> (u64, u64) {
    iso.post_snapshot(encode_snapshot(&base_snapshot(1, far())));
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__rs_a").unwrap(), serde_json::Value::Null);
    assert_eq!(iso.probe("__rs_b").unwrap(), serde_json::Value::Null);
    let drained = iso.drain_interacts();
    match &drained[..] {
        [InteractReq::WalkNear {
            x: 2820,
            z: 3556,
            level: 0,
            radius: 1,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id: first,
        }, InteractReq::WalkNear {
            x: 2820,
            z: 3556,
            level: 0,
            radius: 1,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id: second,
        }] => {
            assert_ne!(*first, 0);
            assert_ne!(*second, 0);
            assert_ne!(first, second);
            (*first, *second)
        }
        other => panic!("unexpected interacts: {other:?}"),
    }
}

#[test]
fn isolate_two_same_target_begins_delayed_first_id_does_not_settle_second() {
    let iso = LoadIsolate::spawn(overlapping_walk_src(), LoadShape::CompatClass, vec![]).unwrap();
    let (first_id, second_id) = park_two_walks(&iso);
    iso.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(2, far()),
        fail_native(1, 0, first_id, 2820, 3556, 1),
    ));
    iso.on_game_tick(2);
    assert_eq!(
        iso.probe("__rs_b").unwrap(),
        serde_json::Value::Null,
        "delayed first request id must not settle the later wait"
    );
    assert_eq!(iso.probe("__rs_a").unwrap(), serde_json::Value::Null);
    iso.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(3, far()),
        fail_native(2, 0, second_id, 2820, 3556, 1),
    ));
    iso.on_game_tick(3);
    assert_eq!(iso.probe("__rs_b").unwrap(), false);
    iso.join();
}

#[test]
fn isolate_held_matching_refusal_survives_later_old_outcome_until_resume() {
    let iso = LoadIsolate::spawn(overlapping_walk_src(), LoadShape::CompatClass, vec![]).unwrap();
    let (first_id, second_id) = park_two_walks(&iso);

    let mut held = base_snapshot(2, far());
    held.hold = true;
    iso.post_snapshot(encode_snapshot_with_native(
        &held,
        fail_native(1, 0, second_id, 2820, 3556, 1),
    ));
    iso.on_game_tick(2);
    assert_eq!(
        iso.probe("__rs_b").unwrap(),
        serde_json::Value::Null,
        "guardian hold must not settle the current JavaScript wait"
    );

    let mut still_held = base_snapshot(3, far());
    still_held.hold = true;
    iso.post_snapshot(encode_snapshot_with_native(
        &still_held,
        fail_native(2, 0, first_id, 2820, 3556, 1),
    ));
    iso.on_game_tick(3);
    assert_eq!(
        iso.probe("__rs_b").unwrap(),
        serde_json::Value::Null,
        "a later held snapshot still must not settle JavaScript"
    );

    iso.post_snapshot(encode_snapshot(&base_snapshot(4, far())));
    iso.on_game_tick(4);
    assert_eq!(
        iso.probe("__rs_b").unwrap(),
        false,
        "the matching refusal observed under hold must settle on the first eligible tick"
    );
    iso.join();
}

#[test]
fn isolate_paused_matching_refusal_survives_later_old_outcome_until_resume() {
    let iso = LoadIsolate::spawn(overlapping_walk_src(), LoadShape::CompatClass, vec![]).unwrap();
    let (first_id, second_id) = park_two_walks(&iso);
    iso.pause();

    iso.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(2, far()),
        fail_native(1, 0, second_id, 2820, 3556, 1),
    ));
    iso.on_game_tick(2);
    iso.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(3, far()),
        fail_native(2, 0, first_id, 2820, 3556, 1),
    ));
    iso.on_game_tick(3);
    assert_eq!(
        iso.probe("__rs_b").unwrap(),
        serde_json::Value::Null,
        "Pause must not settle the current JavaScript wait"
    );

    iso.resume();
    iso.on_game_tick(4);
    assert_eq!(
        iso.probe("__rs_b").unwrap(),
        false,
        "the matching refusal observed while paused must settle after resume"
    );
    iso.join();
}

#[test]
fn isolate_stop_start_does_not_consume_prior_request_id() {
    let iso1 = LoadIsolate::spawn(walk_src(300_000), LoadShape::CompatClass, vec![]).unwrap();
    let old_id = park_walk(&iso1);
    iso1.join();
    let leftover = fail_native(4, 1, old_id, 2820, 3556, 1);
    let iso2 = LoadIsolate::spawn(walk_src(300_000), LoadShape::CompatClass, vec![]).unwrap();
    iso2.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(1, far()),
        leftover,
    ));
    iso2.on_game_tick(1);
    assert_eq!(
        iso2.probe("__rs_ok").unwrap(),
        serde_json::Value::Null,
        "leftover host outcome must not settle a new isolate wait"
    );
    let new_id = match &iso2.drain_interacts()[..] {
        [InteractReq::WalkNear { request_id, .. }] => *request_id,
        other => panic!("unexpected interacts: {other:?}"),
    };
    assert_ne!(new_id, old_id);
    iso2.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(2, far()),
        fail_native(5, 1, old_id, 2820, 3556, 1),
    ));
    iso2.on_game_tick(2);
    assert_eq!(
        iso2.probe("__rs_ok").unwrap(),
        serde_json::Value::Null,
        "late old-worker request id must not settle the new isolate wait"
    );
    iso2.join();
}

fn walk_to_src(call: &str) -> String {
    format!(
        r#"
import {{ Traversal }} from '../../api/walking/Traversal.js';
export default class T extends LoopingBot {{
    async loop() {{
        globalThis.__rs_ok = null;
        globalThis.__rs_ok = await {call};
    }}
}}
"#
    )
}

fn chaos_here() -> TileInput {
    TileInput {
        x: 3104,
        z: 9909,
        level: 0,
    }
}

fn park_walk_to(iso: &LoadIsolate, here: TileInput, expect_walk_near: bool) -> u64 {
    iso.post_snapshot(encode_snapshot(&base_snapshot(1, here)));
    iso.on_game_tick(1);
    assert_eq!(
        iso.probe("__rs_ok").unwrap(),
        serde_json::Value::Null,
        "queued native route is not arrival"
    );
    let drained = iso.drain_interacts();
    assert!(
        drained
            .iter()
            .all(|req| !matches!(req, InteractReq::WalkTo { .. })),
        "Traversal.walkTo must not queue scene WalkTo: {drained:?}"
    );
    let request_id = if expect_walk_near {
        match &drained[..] {
            [InteractReq::WalkNear {
                x: 3096,
                z: 9868,
                level: 0,
                radius: 2,
                allow_teleports: false,
                allow_wilderness: true,
                allow_bank_fetch: true,
                request_id,
            }] => *request_id,
            other => panic!("expected WalkNear radius 2 for world dest: {other:?}"),
        }
    } else {
        match &drained[..] {
            [InteractReq::Walk {
                x: 3096,
                z: 9868,
                level: 0,
                allow_teleports: false,
                allow_wilderness: true,
                allow_bank_fetch: true,
                request_id,
            }] => *request_id,
            other => panic!("expected Walk (radius 0) for world dest: {other:?}"),
        }
    };
    assert_ne!(request_id, 0, "isolate must allocate a walk request id");
    request_id
}

#[test]
fn walk_to_off_scene_world_dest_queues_native_walk_not_scene_walk_to() {
    let iso = LoadIsolate::spawn(
        walk_to_src("Traversal.walkTo({ x: 3096, z: 9868, level: 0 })"),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let request_id = park_walk_to(&iso, chaos_here(), false);
    iso.post_snapshot(encode_snapshot(&base_snapshot(2, chaos_here())));
    iso.on_game_tick(2);
    assert_eq!(
        iso.probe("__rs_ok").unwrap(),
        serde_json::Value::Null,
        "pending native walk must not settle without arrival or matching refusal"
    );
    iso.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(3, chaos_here()),
        fail_native(1, 0, request_id, 3096, 9868, 0),
    ));
    iso.on_game_tick(3);
    assert_eq!(iso.probe("__rs_ok").unwrap(), false);
    iso.join();
}

#[test]
fn walk_to_explicit_nonzero_radius_queues_walk_near() {
    let iso = LoadIsolate::spawn(
        walk_to_src(
            "Traversal.walkTo({ x: 3096, z: 9868, level: 0 }, { radius: 2, timeoutMs: 300000 })",
        ),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    park_walk_to(&iso, chaos_here(), true);
    iso.join();
}

#[test]
fn walk_to_stays_pending_until_posted_arrival() {
    let iso = LoadIsolate::spawn(
        walk_to_src(
            "Traversal.walkTo({ x: 2820, z: 3556, level: 0 }, { radius: 1, timeoutMs: 300000 })",
        ),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    iso.post_snapshot(encode_snapshot(&base_snapshot(1, far())));
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__rs_ok").unwrap(), serde_json::Value::Null);
    match &iso.drain_interacts()[..] {
        [InteractReq::WalkNear {
            x: 2820,
            z: 3556,
            level: 0,
            radius: 1,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id,
        }] => assert_ne!(*request_id, 0),
        other => panic!("walkTo radius 1 must queue WalkNear: {other:?}"),
    }
    iso.post_snapshot(encode_snapshot(&base_snapshot(2, near())));
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__rs_ok").unwrap(), true);
    iso.join();
}

#[test]
fn walk_to_same_coord_wrong_plane_is_not_arrival() {
    let iso = LoadIsolate::spawn(
        walk_to_src(
            "Traversal.walkTo({ x: 3104, z: 9909, level: 0 }, { radius: 0, timeoutMs: 300000 })",
        ),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let other_plane = TileInput {
        x: 3104,
        z: 9909,
        level: 1,
    };
    iso.post_snapshot(encode_snapshot(&base_snapshot(1, other_plane)));
    iso.on_game_tick(1);
    assert_eq!(
        iso.probe("__rs_ok").unwrap(),
        serde_json::Value::Null,
        "same x/z on another plane is not arrival"
    );
    match &iso.drain_interacts()[..] {
        [InteractReq::Walk {
            x: 3104,
            z: 9909,
            level: 0,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id,
        }] => assert_ne!(*request_id, 0),
        other => panic!("wrong-plane dest must still queue native Walk: {other:?}"),
    }
    iso.post_snapshot(encode_snapshot(&base_snapshot(2, other_plane)));
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__rs_ok").unwrap(), serde_json::Value::Null);
    iso.join();
}

#[test]
fn walk_to_stop_start_does_not_consume_prior_request_id() {
    let src = walk_to_src(
        "Traversal.walkTo({ x: 2820, z: 3556, level: 0 }, { radius: 1, timeoutMs: 300000 })",
    );
    let iso1 = LoadIsolate::spawn(src.clone(), LoadShape::CompatClass, vec![]).unwrap();
    iso1.post_snapshot(encode_snapshot(&base_snapshot(1, far())));
    iso1.on_game_tick(1);
    assert_eq!(iso1.probe("__rs_ok").unwrap(), serde_json::Value::Null);
    let old_id = match &iso1.drain_interacts()[..] {
        [InteractReq::WalkNear { request_id, .. }] => *request_id,
        other => panic!("unexpected interacts: {other:?}"),
    };
    iso1.join();
    let iso2 = LoadIsolate::spawn(src, LoadShape::CompatClass, vec![]).unwrap();
    iso2.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(1, far()),
        fail_native(4, 1, old_id, 2820, 3556, 1),
    ));
    iso2.on_game_tick(1);
    assert_eq!(
        iso2.probe("__rs_ok").unwrap(),
        serde_json::Value::Null,
        "leftover host outcome must not settle a new Traversal.walkTo wait"
    );
    let new_id = match &iso2.drain_interacts()[..] {
        [InteractReq::WalkNear { request_id, .. }] => *request_id,
        other => panic!("unexpected interacts: {other:?}"),
    };
    assert_ne!(new_id, old_id);
    iso2.post_snapshot(encode_snapshot_with_native(
        &base_snapshot(2, far()),
        fail_native(5, 1, old_id, 2820, 3556, 1),
    ));
    iso2.on_game_tick(2);
    assert_eq!(
        iso2.probe("__rs_ok").unwrap(),
        serde_json::Value::Null,
        "late old-worker request id must not settle the new walkTo wait"
    );
    iso2.join();
}
