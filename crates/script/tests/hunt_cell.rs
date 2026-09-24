//! Offline Cell isolate policy tests. No LIVE.
//!
//! These lock the architecture corrections: KBD abort is not yield-false,
//! inv 1590 inside the cell does not complete, door and Velrak distance are
//! live Chebyshev, unlock consumes 1591 not keyItem.id, inside is not
//! cell-later, and walk-to carries no flags. They do not mirror phases.

use script::hunt_cell::{
    self, cell_deadline_remaining_ms, cell_force_bound_reached, cell_token_alive, set_observation,
    CellInv, CellLoc, CellNpc, CellObservation, DOOR_MS, VELRAK_WALK_MS, WALK_LEG_MS,
};
use script::hunt_fight::Tile;
use script::isolate_fb::{
    encode_snapshot, ItemRowInput, SceneEntityInput, SnapshotInput, SnapshotReader, TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use serde_json::{json, Value};

fn reset() {
    hunt_cell::on_reset();
}

fn tile(x: i32, z: i32, level: i32) -> Tile {
    Tile { x, z, level }
}

fn outside() -> Tile {
    tile(2900, 9600, 0)
}

fn door_stand() -> Tile {
    tile(2931, 9690, 0)
}

fn inside_stand() -> Tile {
    tile(2931, 9689, 0)
}

fn cell_here() -> Tile {
    tile(2931, 9686, 0)
}

fn lair_here() -> Tile {
    tile(50, 50, 0)
}

fn boxes() -> Value {
    json!([{ "minX": 40, "maxX": 60, "minZ": 40, "maxZ": 60, "level": 0 }])
}

fn inv(id: i32, count: i32, name: &str, slot: i32, has_slot: bool) -> CellInv {
    CellInv {
        id,
        count,
        name: name.to_string(),
        slot,
        has_slot,
    }
}

fn dusty() -> CellInv {
    inv(1590, 1, "Dusty key", 1, true)
}

fn jail_key() -> CellInv {
    inv(1591, 1, "Jail key", 4, true)
}

fn loc(id: i32, at: Tile) -> CellLoc {
    CellLoc { id, tile: at }
}

fn velrak(index: i32, at: Tile, action: &str) -> CellNpc {
    CellNpc {
        index,
        name: "Velrak the explorer".into(),
        actions: vec![action.into()],
        tile: at,
    }
}

fn obs(here: Option<Tile>) -> CellObservation {
    CellObservation {
        here,
        ingame: true,
        hold: false,
        ours: false,
        inv: vec![],
        locs: vec![],
        npcs: vec![],
        chat_modal_id: -1,
        chat_continue: false,
        chat_options: vec![],
    }
}

fn site(extra: Value) -> Value {
    let mut base = json!({
        "key": "taverley-blue",
        "keyItem": { "id": 1590, "name": "Dusty key" },
        "boxes": boxes(),
    });
    if let Some(obj) = extra.as_object() {
        for (k, v) in obj {
            base[k] = v.clone();
        }
    }
    base
}

fn unkeyed(key: &str) -> Value {
    site(json!({ "key": key, "keyItem": null }))
}

fn begin() -> u64 {
    hunt_cell::dispatch(&json!({ "op": "begin" }))["token"]
        .as_u64()
        .expect("token")
}

fn call(token: u64, mut p: Value, reply: Option<Value>) -> Value {
    p["op"] = json!("next");
    p["token"] = json!(token);
    if let Some(r) = reply {
        p["reply"] = r;
    }
    hunt_cell::dispatch(&p)
}

fn walk_reply() -> Value {
    json!({ "queued": true, "walkToken": 9001 })
}

fn queued() -> Value {
    json!({ "queued": true })
}

fn kind(step: &Value) -> &str {
    step["kind"].as_str().unwrap_or("")
}

fn assert_yield(step: &Value, value: bool) {
    assert_eq!(kind(step), "yield", "{step}");
    assert_eq!(step["value"], value, "{step}");
    assert!(step.get("x").is_none(), "{step}");
    assert_ne!(step["reason"], "cell-later");
}

fn assert_abort(step: &Value, reason: &str) {
    assert_eq!(kind(step), "aborted", "{step}");
    assert_eq!(step["reason"], reason, "{step}");
    assert!(step.get("value").is_none(), "abort is not a yield: {step}");
    assert_ne!(kind(step), "yield");
}

fn no_walk_flags(step: &Value) {
    assert!(step.get("radius").is_none(), "{step}");
    assert!(step.get("allow_teleports").is_none(), "{step}");
    assert!(step.get("allow_wilderness").is_none(), "{step}");
    assert!(step.get("allow_bank_fetch").is_none(), "{step}");
    assert!(step.get("request_id").is_none(), "{step}");
    assert!(step.get("flags").is_none(), "{step}");
}

#[test]
fn heroes_gutanoth_and_brimhaven_yield_true_with_no_ops() {
    for key in [
        "heroes-blue",
        "gutanoth-blue",
        "brimhaven-iron",
        "brimhaven-steel",
    ] {
        reset();
        set_observation(obs(Some(outside())));
        let step = call(begin(), unkeyed(key), None);
        assert_yield(&step, true);
        assert_ne!(kind(&step), "leave");
        assert_ne!(kind(&step), "key");
        assert_ne!(kind(&step), "npc");
    }
}

#[test]
fn kbd_null_key_item_aborts_and_does_not_yield_false() {
    reset();
    set_observation(obs(Some(outside())));
    let step = call(
        begin(),
        site(json!({ "key": "kbd-lair", "keyItem": null })),
        None,
    );
    assert_abort(&step, "kbd-later");
}

#[test]
fn projected_route_and_kbd_locs_abort_even_when_unkeyed() {
    reset();
    set_observation(obs(Some(outside())));
    let step = call(
        begin(),
        site(json!({ "key": "heroes-blue", "keyItem": null, "route": {} })),
        None,
    );
    assert_abort(&step, "kbd-later");

    for id in [1765, 1766, 1816, 1817] {
        reset();
        let mut seen = obs(Some(outside()));
        seen.locs = vec![loc(id, outside())];
        set_observation(seen);
        let step = call(begin(), unkeyed("heroes-blue"), None);
        assert_abort(&step, "kbd-later");
        assert_ne!(kind(&step), "yield");
    }

    reset();
    let mut seen = obs(Some(outside()));
    seen.locs = vec![loc(2631, door_stand())];
    set_observation(seen);
    let step = call(begin(), unkeyed("heroes-blue"), None);
    assert_yield(&step, true);
}

#[test]
fn dusty_key_outside_yields_true_before_leave_even_inside_the_lair() {
    reset();
    let mut seen = obs(Some(lair_here()));
    seen.inv = vec![dusty()];
    set_observation(seen);
    let step = call(begin(), site(json!({})), None);
    assert_yield(&step, true);
    assert_ne!(kind(&step), "leave");
    assert_ne!(kind(&step), "key");
}

#[test]
fn dusty_key_inside_the_cell_does_not_complete_or_talk_or_fetch() {
    reset();
    let mut seen = obs(Some(cell_here()));
    seen.inv = vec![dusty(), jail_key()];
    seen.npcs = vec![velrak(7, cell_here(), "Talk-to")];
    set_observation(seen);
    let step = call(begin(), site(json!({})), None);
    assert_ne!(kind(&step), "yield", "1590 inside is not done: {step}");
    assert_ne!(step["value"], true);
    assert_ne!(kind(&step), "aborted", "{step}");
    assert_ne!(step["reason"], "cell-later");
    assert_ne!(kind(&step), "key", "do not acquireKey while inside: {step}");
    assert_ne!(
        kind(&step),
        "leave",
        "do not leaveLair while inside: {step}"
    );
    assert_ne!(
        kind(&step),
        "npc",
        "do not talk while 1590 is already held: {step}"
    );
    assert_ne!(kind(&step), "use-on");
    assert_eq!(kind(&step), "walk", "{step}");
    assert_eq!(step["x"], 2931);
    assert_eq!(step["z"], 9689);
    assert_ne!(
        step["z"], 9690,
        "do not walk the outside door for a sealed-in leave"
    );
}

#[test]
fn jail_key_and_bank_and_cert_are_not_completion() {
    reset();
    let mut seen = obs(Some(outside()));
    seen.inv = vec![jail_key()];
    set_observation(seen);
    let step = call(begin(), site(json!({})), None);
    assert_ne!(kind(&step), "yield", "{step}");
    assert_eq!(
        kind(&step),
        "walk-near",
        "held 1591 approaches the door, not done"
    );
    assert_eq!(step["x"], 2931);
    assert_eq!(step["z"], 9690);
    assert_ne!(step["z"], 9689, "unlock does not walk the inside stand");

    reset();
    let mut seen = obs(Some(outside()));
    seen.inv = vec![inv(999, 1, "Dusty key", 2, true)];
    set_observation(seen);
    let step = call(begin(), site(json!({})), None);
    assert_ne!(step["value"], true, "name is not the id: {step}");
}

#[test]
fn outside_without_jail_key_calls_acquire_key_before_any_door_walk() {
    reset();
    set_observation(obs(Some(outside())));
    let step = call(begin(), site(json!({})), None);
    assert_eq!(kind(&step), "key", "{step}");
    assert!(
        step.get("x").is_none(),
        "do not walk 2931,9690 before acquireKey: {step}"
    );
    assert_ne!(kind(&step), "walk");
    assert_ne!(kind(&step), "walk-near");
    assert_ne!(kind(&step), "npc");
    assert_ne!(step["action"], "Attack");
}

#[test]
fn lair_without_the_dusty_key_leaves_and_a_false_left_yields_false() {
    reset();
    set_observation(obs(Some(lair_here())));
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_eq!(kind(&step), "leave", "{step}");
    assert_ne!(kind(&step), "key");
    let step = call(token, site(json!({})), Some(json!({ "left": false })));
    assert_yield(&step, false);
}

#[test]
fn fourth_attempt_yields_false_and_a_walk_reclick_does_not_burn_one() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let proj = site(json!({}));
    let mut keys = 0;
    let mut reply = None;
    for _ in 0..6 {
        let step = call(token, proj.clone(), reply);
        if kind(&step) == "yield" {
            assert_eq!(step["value"], false, "{step}");
            assert_eq!(keys, 3, "three acquireKey failures, then false");
            return;
        }
        assert_eq!(kind(&step), "key", "{step}");
        keys += 1;
        reply = Some(json!({ "held": false }));
    }
    panic!("never yielded");
}

#[test]
fn velrak_walk_to_reclick_does_not_yield_false_or_add_flags() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let proj = site(json!({}));
    assert_eq!(kind(&call(token, proj.clone(), None)), "key");

    let mut seen = obs(Some(cell_here()));
    seen.npcs = vec![velrak(3, tile(2931, 9680, 0), "Talk-to")];
    set_observation(seen);
    let step = call(token, proj.clone(), Some(json!({ "held": true })));
    assert_eq!(kind(&step), "delay-ticks", "{step}");
    let step = call(token, proj.clone(), None);
    assert_eq!(kind(&step), "walk-to", "{step}");
    assert_eq!(step["x"], 2931);
    assert_eq!(step["z"], 9680);
    no_walk_flags(&step);
    assert_ne!(kind(&step), "walk");
    assert_ne!(kind(&step), "walk-near");
    let window = cell_deadline_remaining_ms(token).expect("velrak window");
    assert!(
        window <= VELRAK_WALK_MS as i64 && window > 15_000,
        "{window}"
    );

    cell_force_bound_reached(token);
    let step = call(token, proj.clone(), None);
    assert_eq!(
        kind(&step),
        "walk-to",
        "stall re-click is not a failed attempt: {step}"
    );
    no_walk_flags(&step);
    assert_ne!(kind(&step), "yield");
    assert_ne!(kind(&step), "key");
}

#[test]
fn posted_npc_distance_zero_does_not_skip_the_walk() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let proj = site(json!({}));
    assert_eq!(kind(&call(token, proj.clone(), None)), "key");

    let talk = vec!["Talk-to".to_string()];
    let npc = SceneEntityInput {
        index: 9,
        id: 802,
        name: Some("Velrak the explorer"),
        x: 2931,
        z: 9680,
        level: 0,
        distance: 0,
        health: 0,
        max_health: 0,
        in_combat: false,
        animating: false,
        actions: &talk,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 1,
        nx: 0,
        nz: 0,
    };
    let bytes = encode_snapshot(&snap_scene(cell_here(), &[], &[], &[npc]));
    let reader = SnapshotReader::from_bytes(&bytes).unwrap();
    script::observed::apply(&reader);
    let step = call(token, proj.clone(), Some(json!({ "held": true })));
    assert_eq!(kind(&step), "delay-ticks", "{step}");
    let step = call(token, proj, None);
    assert_eq!(
        kind(&step),
        "walk-to",
        "posted distance 0 must not skip the walk: {step}"
    );
    assert_eq!(step["z"], 9680);
    assert_ne!(kind(&step), "npc");
    no_walk_flags(&step);
}

#[test]
fn unlock_uses_jail_key_1591_not_key_item_id_and_live_chebyshev() {
    reset();
    let near = tile(2930, 9688, 0);
    let far = tile(2900, 9690, 0);
    let mut seen = obs(Some(door_stand()));
    seen.inv = vec![jail_key(), inv(2631, 1, "Highwayman mask", 6, true)];
    seen.locs = vec![loc(2631, far), loc(2623, near), loc(2631, near)];
    set_observation(seen);
    let token = begin();
    let proj = site(json!({}));
    let step = call(token, proj.clone(), None);
    assert_eq!(kind(&step), "delay-ticks", "{step}");
    assert_eq!(step["n"], 2);
    assert_ne!(kind(&step), "use-on");
    assert_ne!(kind(&step), "walk");
    assert_ne!(step["z"], 9689);

    let step = call(token, proj.clone(), None);
    assert_eq!(kind(&step), "use-on", "{step}");
    assert_eq!(
        step["id"], 1591,
        "not keyItem.id 1590 and not loc 2631: {step}"
    );
    assert_eq!(step["name"], "Jail key");
    assert_eq!(step["slot"], 4);
    assert_eq!(step["x"], near.x);
    assert_eq!(step["z"], near.z);
    assert!(step.get("locId").is_none(), "{step}");
    assert_ne!(kind(&step), "obj");
    assert_ne!(kind(&step), "loc");
    let window = cell_deadline_remaining_ms(token).expect("door window");
    assert!(window <= DOOR_MS as i64 && window > 6_000, "{window}");

    let step = call(token, proj, Some(queued()));
    assert_ne!(kind(&step), "yield", "queued is not inside: {step}");
    assert_ne!(step["value"], true);
}

#[test]
fn posted_loc_distance_does_not_pick_the_door() {
    reset();
    let talk = Vec::<String>::new();
    let far = SceneEntityInput {
        index: 1,
        id: 2631,
        name: Some("Door"),
        x: 2900,
        z: 9690,
        level: 0,
        distance: 0,
        health: 0,
        max_health: 0,
        in_combat: false,
        animating: false,
        actions: &talk,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 0,
        nx: 0,
        nz: 0,
    };
    let near = SceneEntityInput {
        index: 2,
        id: 2631,
        name: Some("Door"),
        x: 2930,
        z: 9688,
        level: 0,
        distance: 99,
        health: 0,
        max_health: 0,
        in_combat: false,
        animating: false,
        actions: &talk,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 0,
        nx: 0,
        nz: 0,
    };
    let other_level = SceneEntityInput {
        index: 3,
        id: 2631,
        name: Some("Door"),
        x: 2931,
        z: 9690,
        level: 1,
        distance: 0,
        health: 0,
        max_health: 0,
        in_combat: false,
        animating: false,
        actions: &talk,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 0,
        nx: 0,
        nz: 0,
    };
    let jail = ItemRowInput {
        name: Some("Jail key"),
        count: 1,
        id: 1591,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot: 4,
    };
    let bytes = encode_snapshot(&snap_scene(
        door_stand(),
        &[jail],
        &[far, near, other_level],
        &[],
    ));
    let reader = SnapshotReader::from_bytes(&bytes).unwrap();
    script::observed::apply(&reader);
    let token = begin();
    let proj = site(json!({}));
    assert_eq!(kind(&call(token, proj.clone(), None)), "delay-ticks");
    let step = call(token, proj, None);
    assert_eq!(kind(&step), "use-on", "{step}");
    assert_eq!(
        step["x"], 2930,
        "live near tile, not posted distance 0: {step}"
    );
    assert_eq!(step["z"], 9688);
    assert_eq!(step["id"], 1591);
    assert_ne!(step["level"], 1);
}

#[test]
fn missing_jail_key_name_or_slot_does_not_emit_use_on() {
    for broken in [
        inv(1591, 1, "", 4, true),
        inv(1591, 1, "Jail key", -1, false),
    ] {
        reset();
        let mut seen = obs(Some(door_stand()));
        seen.inv = vec![broken];
        seen.locs = vec![loc(2631, tile(2930, 9688, 0))];
        set_observation(seen);
        let token = begin();
        let proj = site(json!({}));
        assert_eq!(kind(&call(token, proj.clone(), None)), "delay-ticks");
        let step = call(token, proj, None);
        assert_ne!(kind(&step), "use-on", "{step}");
        assert_ne!(kind(&step), "obj");
        assert_ne!(step["id"], 1590);
    }
}

#[test]
fn missing_door_within_live_five_does_not_use_on_or_open_another_loc() {
    reset();
    let mut seen = obs(Some(door_stand()));
    seen.inv = vec![jail_key()];
    seen.locs = vec![loc(2623, door_stand()), loc(2631, tile(2800, 9600, 0))];
    set_observation(seen);
    let token = begin();
    let proj = site(json!({}));
    let mut saw_use_on = false;
    let mut saw_loc = false;
    for _ in 0..8 {
        let step = call(token, proj.clone(), None);
        if kind(&step) == "yield" {
            assert_eq!(step["value"], false, "{step}");
            assert!(!saw_use_on);
            assert!(!saw_loc);
            return;
        }
        saw_use_on |= kind(&step) == "use-on";
        saw_loc |= kind(&step) == "loc";
        assert_ne!(kind(&step), "obj");
    }
    panic!("missing door did not fail closed");
}

#[test]
fn door_approach_failure_does_not_fall_through_to_use_on() {
    reset();
    let mut seen = obs(Some(outside()));
    seen.inv = vec![jail_key()];
    set_observation(seen);
    let token = begin();
    let proj = site(json!({}));
    let step = call(token, proj.clone(), None);
    assert_eq!(kind(&step), "walk-near", "{step}");
    assert_eq!(step["radius"], 1);
    assert_eq!(step["z"], 9690);
    let window = cell_deadline_remaining_ms(token).expect("walk leg");
    assert!(window <= WALK_LEG_MS as i64 && window > 290_000, "{window}");

    cell_force_bound_reached(token);
    let step = call(token, proj.clone(), Some(walk_reply()));
    assert_eq!(
        kind(&step),
        "walk-near",
        "failed approach does not exact-walk: {step}"
    );
    assert_ne!(kind(&step), "walk");
    assert_ne!(kind(&step), "use-on");
    assert_ne!(kind(&step), "yield");
}

#[test]
fn occupied_exact_stand_does_not_use_on_from_the_neighbour() {
    reset();
    let mut seen = obs(Some(tile(2932, 9690, 0)));
    seen.inv = vec![jail_key()];
    seen.locs = vec![loc(2631, tile(2932, 9688, 0))];
    set_observation(seen);
    let token = begin();
    let proj = site(json!({}));
    let step = call(token, proj.clone(), None);
    assert_eq!(kind(&step), "walk", "{step}");
    assert_eq!(step["x"], 2931);
    assert_eq!(step["z"], 9690);
    assert!(
        step.get("radius").is_none(),
        "walk radius is hardcoded 0 in JS: {step}"
    );

    cell_force_bound_reached(token);
    let step = call(token, proj, Some(walk_reply()));
    assert_ne!(kind(&step), "use-on", "{step}");
    assert_ne!(kind(&step), "yield");
    assert_ne!(step["value"], false);
}

#[test]
fn occupied_inside_stand_still_opens_and_exact_failure_is_not_yield_false() {
    reset();
    let mut seen = obs(Some(cell_here()));
    seen.locs = vec![loc(2631, cell_here())];
    set_observation(seen);
    let token = begin();
    let proj = site(json!({}));
    let step = call(token, proj.clone(), None);
    assert_eq!(kind(&step), "walk", "{step}");
    assert_eq!(step["z"], 9689);

    cell_force_bound_reached(token);
    let step = call(token, proj.clone(), Some(walk_reply()));
    assert_eq!(kind(&step), "walk-near", "{step}");
    assert_eq!(step["radius"], 2);
    assert_ne!(kind(&step), "yield");
    assert_ne!(kind(&step), "use-on");

    cell_force_bound_reached(token);
    let step = call(token, proj.clone(), Some(walk_reply()));
    assert_eq!(
        kind(&step),
        "delay-ticks",
        "walk-near false still opens: {step}"
    );
    assert_ne!(kind(&step), "yield");
    let step = call(token, proj, None);
    assert_eq!(kind(&step), "loc", "{step}");
    assert_eq!(step["action"], "Open");
    assert_eq!(step["id"], 2631);
    assert_ne!(kind(&step), "use-on");
}

#[test]
fn refused_open_fails_the_pump_and_does_not_use_on() {
    reset();
    let mut seen = obs(Some(inside_stand()));
    seen.locs = vec![loc(2631, inside_stand())];
    set_observation(seen);
    let token = begin();
    let proj = site(json!({}));
    assert_eq!(kind(&call(token, proj.clone(), None)), "delay-ticks");
    let step = call(token, proj.clone(), None);
    assert_eq!(kind(&step), "loc", "{step}");
    assert_eq!(step["action"], "Open");
    let step = call(token, proj, Some(json!({ "queued": false })));
    assert_yield(&step, false);
}

#[test]
fn dialog_answer_is_the_posted_slot_and_closed_is_not_the_handoff() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let proj = site(json!({}));
    assert_eq!(kind(&call(token, proj.clone(), None)), "key");

    let mut seen = obs(Some(cell_here()));
    seen.npcs = vec![velrak(4, cell_here(), "Talk-to")];
    seen.chat_modal_id = 12;
    seen.chat_options = vec![
        "Yes please!".into(),
        "So... do you know anywhere good to explore?".into(),
    ];
    set_observation(seen);
    assert_eq!(
        kind(&call(token, proj.clone(), Some(json!({ "held": true })))),
        "delay-ticks"
    );
    let step = call(token, proj.clone(), None);
    assert_eq!(kind(&step), "npc", "{step}");
    assert_eq!(step["action"], "Talk-to");
    assert_eq!(step["index"], 4);
    assert_ne!(step["action"], "Attack");

    let step = call(token, proj.clone(), Some(queued()));
    assert_eq!(kind(&step), "answer", "{step}");
    assert_eq!(step["option"], 2, "first fragment wins, 1-based: {step}");
    assert!(step.get("text").is_none());
    assert_ne!(kind(&step), "close-modal");

    let mut seen = obs(Some(cell_here()));
    seen.chat_modal_id = 12;
    seen.chat_options = vec!["Nope".into(), "".into()];
    set_observation(seen);
    let step = call(token, proj.clone(), None);
    assert_eq!(kind(&step), "answer", "{step}");
    assert_eq!(
        step["option"], 2,
        "empty last slot stays the posted index: {step}"
    );

    let mut seen = obs(Some(cell_here()));
    seen.chat_modal_id = -1;
    set_observation(seen);
    let step = call(token, proj.clone(), None);
    assert_ne!(
        kind(&step),
        "yield",
        "dialog closed is not the handoff: {step}"
    );
    assert_ne!(step["value"], true);
    assert_ne!(kind(&step), "close-modal");
}

#[test]
fn dialog_drive_stops_at_120_steps_not_120_seconds() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let proj = site(json!({}));
    assert_eq!(kind(&call(token, proj.clone(), None)), "key");
    let mut seen = obs(Some(cell_here()));
    seen.npcs = vec![velrak(1, cell_here(), "talk")];
    seen.chat_continue = true;
    seen.chat_modal_id = 3;
    set_observation(seen);
    assert_eq!(
        kind(&call(token, proj.clone(), Some(json!({ "held": true })))),
        "delay-ticks"
    );
    let step = call(token, proj.clone(), None);
    assert_eq!(kind(&step), "npc");
    assert_eq!(step["action"], "talk");
    let step = call(token, proj.clone(), Some(queued()));
    assert_eq!(kind(&step), "continue", "{step}");
    let mut continues = 1;
    for _ in 0..130 {
        let step = call(token, proj.clone(), None);
        if kind(&step) == "continue" {
            continues += 1;
            continue;
        }
        assert_eq!(continues, 120, "stopped after {continues}: {step}");
        assert_ne!(kind(&step), "yield");
        assert_ne!(kind(&step), "close-modal");
        assert_ne!(step["value"], true);
        return;
    }
    panic!("dialog did not stop");
}

#[test]
fn inside_with_dusty_after_key_leaves_instead_of_talking() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let proj = site(json!({}));
    assert_eq!(kind(&call(token, proj.clone(), None)), "key");
    let mut seen = obs(Some(cell_here()));
    seen.inv = vec![dusty()];
    seen.npcs = vec![velrak(1, cell_here(), "Talk-to")];
    set_observation(seen);
    let step = call(token, proj, Some(json!({ "held": false })));
    assert_ne!(kind(&step), "yield", "{step}");
    assert_ne!(kind(&step), "npc");
    assert_ne!(kind(&step), "key");
    assert_ne!(step["reason"], "cell-later");
    assert_eq!(kind(&step), "walk");
    assert_eq!(step["z"], 9689);
}

#[test]
fn begin_allocates_a_new_token_and_does_not_abort_the_existing_one() {
    reset();
    set_observation(obs(Some(outside())));
    let first = begin();
    let second = begin();
    assert_ne!(first, second);
    assert!(cell_token_alive(first));
    assert!(cell_token_alive(second));
    assert_yield(&call(first, unkeyed("heroes-blue"), None), true);
    assert_yield(&call(second, unkeyed("gutanoth-blue"), None), true);
    assert!(cell_token_alive(first));

    let ended = hunt_cell::dispatch(&json!({ "op": "end", "token": first }));
    assert_eq!(kind(&ended), "ok", "{ended}");
    assert!(!cell_token_alive(first), "end drops that row: {ended}");
    assert!(cell_token_alive(second), "end is not a map clear: {ended}");
    assert_abort(&call(first, unkeyed("heroes-blue"), None), "unknown token");
    let again = hunt_cell::dispatch(&json!({ "op": "end", "token": first }));
    assert_eq!(kind(&again), "ok", "an unknown token is a no-op: {again}");
    assert!(cell_token_alive(second));
}

#[test]
fn pause_emits_wait_and_reset_drops_the_token() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    hunt_cell::on_pause();
    let step = call(token, unkeyed("heroes-blue"), None);
    assert_eq!(kind(&step), "wait", "{step}");
    hunt_cell::on_resume();
    assert_yield(&call(token, unkeyed("heroes-blue"), None), true);
    hunt_cell::on_reset();
    let step = call(token, unkeyed("heroes-blue"), None);
    assert_eq!(kind(&step), "aborted");
    assert_eq!(step["reason"], "unknown token");
}

#[test]
fn hold_does_not_claim_a_held_key() {
    reset();
    let mut seen = obs(Some(outside()));
    seen.inv = vec![dusty()];
    seen.hold = true;
    set_observation(seen);
    let step = call(begin(), site(json!({})), None);
    assert_yield(&step, false);
}

#[test]
fn snapshot_bank_row_is_not_completion() {
    reset();
    let bank = ItemRowInput {
        name: Some("Dusty key"),
        count: 1,
        id: 1590,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot: 0,
    };
    let cert = ItemRowInput {
        name: Some("Dusty key"),
        count: 1,
        id: 999,
        ops: &[],
        noted: true,
        cert: 1590,
        component_id: -1,
        slot: 1,
    };
    let bytes = encode_snapshot(&snap_rows(outside(), &[cert], &[bank]));
    let reader = SnapshotReader::from_bytes(&bytes).unwrap();
    script::observed::apply(&reader);
    let step = call(begin(), site(json!({})), None);
    assert_ne!(
        step["value"], true,
        "bank and cert are not inv 1590: {step}"
    );
    assert_ne!(kind(&step), "yield");
}

#[test]
fn cell_box_edges_are_a_fence_not_a_walk_target() {
    reset();
    let mut seen = obs(Some(tile(2934, 9689, 0)));
    seen.inv = vec![dusty()];
    set_observation(seen);
    let step = call(begin(), site(json!({})), None);
    assert_ne!(kind(&step), "yield", "max corner is inside: {step}");

    reset();
    let mut seen = obs(Some(tile(2931, 9690, 0)));
    seen.inv = vec![dusty()];
    set_observation(seen);
    assert_yield(&call(begin(), site(json!({})), None), true);

    reset();
    let mut seen = obs(Some(tile(2931, 9686, 1)));
    seen.inv = vec![dusty()];
    set_observation(seen);
    assert_yield(&call(begin(), site(json!({})), None), true);
}

#[test]
fn source_does_not_copy_the_forbidden_machines() {
    let src = include_str!("../src/hunt_cell.rs");
    assert!(src.contains("CELL_RUNTIMES"));
    assert!(src.contains("in_area_body(here, 1,"));
    assert!(!src.contains("cell-later"));
    assert!(!src.contains("hunt_key::dispatch"));
    assert!(!src.contains("hunt_leave::dispatch"));
    assert!(!src.contains("hunt_lair::dispatch"));
    assert!(!src.contains("hunt_fight::dispatch"));
    assert!(!src.contains("dialog::dispatch"));
    assert!(!src.contains("keyStatus"));
    assert!(!src.contains(".bank("));
    assert!(!src.contains("bank_side"));
    assert!(!src.contains("equipment"));
    assert!(!src.contains("close-modal"));
    assert!(!src.contains("DirectNavigator"));
    assert!(!src.contains("gap_sw"));
    assert!(!src.contains("bodyOrigin"));
    assert!(!src.contains("size>>1"));
    assert!(!src.contains("size >> 1"));
    assert!(!src.contains(".nx("));
    assert!(!src.contains(".nz("));
    assert!(!src.contains(".distance("));
    assert!(!src.contains("FIGHT_MS"));
    assert!(!src.contains("hunt_key::"));
    assert!(!src.contains("Instant::"));
    assert!(!src.contains("pub const KBD_LOCS"));
    assert!(!src.contains("pub(crate) const KBD_LOCS"));
    assert_eq!(DOOR_MS, 8_000);
    assert_eq!(VELRAK_WALK_MS, 20_000);
    assert_eq!(WALK_LEG_MS, 300_000);
}

#[test]
fn shim_and_bindings_keep_the_yield_shape_and_walk_to_has_no_flags() {
    let bindings = include_str!("../src/load/bindings.rs");
    let cell = bindings.split("api.cellNext").nth(1).expect("cellNext");
    let cell = cell.split("function recordSettlement").next().unwrap();
    assert!(cell.contains("kind: 'yield', value:"));
    assert!(cell.contains("ok: false, error:"));
    assert!(cell.contains("kind: 'aborted'"));
    assert!(cell.contains("status: 'aborted'"));
    assert!(!cell.contains("ok: true, status: 'aborted'"));
    assert!(!bindings.contains("api.cellValidate"));
    let reg = bindings.split("__rs2b0t_cell").nth(1).unwrap();
    let reg = reg.split("__rs2b0t_production").next().unwrap();
    assert!(!reg.contains("SelectedGameData"));
    assert!(!reg.contains("selected_"));

    let dts = include_str!("../src/host_js.rs");
    let step = dts.split("export type CellStep").nth(1).expect("CellStep");
    assert!(step.contains("kind: 'yield'; value: boolean"));
    assert!(step.contains("ok: false; error: string; kind: 'aborted'"));
    assert!(!dts.contains("cellValidate"));

    let js = include_str!("../src/shim/hunting_combat.js");
    let cell_fn = js.split("export async function cell").nth(1).expect("cell");
    let cell_fn = cell_fn.split("export class WalkToSpot").next().unwrap();
    assert!(cell_fn.contains("leaveLair"));
    assert!(cell_fn.contains("acquireKey"));
    assert!(cell_fn.contains("{ left:"));
    assert!(cell_fn.contains("{ held:"));
    assert!(!cell_fn.contains("1590"));
    assert!(!cell_fn.contains("1591"));
    assert!(!cell_fn.contains("2631"));
    assert!(!cell_fn.contains("Inventory"));
    assert!(!cell_fn.contains("close-modal"));
    assert!(!cell_fn.contains("case 'obj'"));
    assert!(!cell_fn.contains("Yes please"));
    let walk_to = cell_fn.split("step.kind === 'walk-to'").nth(1).unwrap();
    let walk_to = walk_to.split("continue;").next().unwrap();
    assert!(walk_to.contains("op: 'walk-to'"));
    assert!(!walk_to.contains("allow_"));
    assert!(!walk_to.contains("radius"));
    assert!(!walk_to.contains("request_id"));
    assert!(!walk_to.contains("__rs2b0t_walk"));
    assert!(!walk_to.contains("beginCellWalk"));
    let use_on = cell_fn.split("case 'use-on':").nth(1).unwrap();
    let use_on = use_on.split("break;").next().unwrap();
    assert!(use_on.contains("kind: 'loc'"));
    assert!(use_on.contains("source_item_id"));
    assert!(!use_on.contains("locId"));
    let begin = js.split("function beginCellWalk").nth(1).unwrap();
    let begin = begin.split("export async function cell").next().unwrap();
    assert!(begin.contains("allow_teleports: false"));
    assert!(begin.contains("allow_wilderness: false"));
    assert!(begin.contains("allow_bank_fetch: false"));
    assert!(!begin.contains("allow_wilderness: true"));
    assert!(cell_fn.contains("beginCellWalk(step, 0)"));
    assert!(cell_fn.contains("radius: 0"));
    assert_eq!(
        cell_fn.matches("cellCall({ op: 'end', token })").count(),
        cell_fn.matches("return ").count() - 1,
        "every return after the begin ends the Rust row: {cell_fn}"
    );

    let acquire = js.split("export async function acquireKey").nth(1).unwrap();
    let acquire = acquire.split("function cellCall").next().unwrap();
    assert!(!acquire.contains("__rs2b0t_cell"));

    let iso = include_str!("../src/load/isolate.rs");
    for hook in ["on_pause", "on_hold", "on_resume", "on_reset"] {
        assert!(
            iso.contains(&format!("hunt_cell::{hook}")),
            "missing {hook}"
        );
        assert!(iso.contains(&format!("hunt_key::{hook}")));
    }
}

#[test]
fn queued_cell_walk_flags_are_false_and_radius_matches() {
    let src = r#"
import { cell } from '../../api/combat/hunting/combat.js';
export default class T extends LoopingBot {
    loop() {
        const host = { log() {}, setStatus() {} };
        const site = {
            key: 'taverley-blue',
            keyItem: { id: 1590 },
            boxes: [{ minX: 40, maxX: 60, minZ: 40, maxZ: 60, level: 0 }],
        };
        cell(host, site);
        globalThis.__ran = true;
    }
}
"#;
    let jail = ItemRowInput {
        name: Some("Jail key"),
        count: 1,
        id: 1591,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot: 3,
    };
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    iso.post_snapshot(encode_snapshot(&snap_rows(
        tile(2900, 9600, 0),
        &[jail],
        &[],
    )));
    iso.on_game_tick(1);
    let ran = iso.probe("globalThis.__ran").unwrap_or(Value::Null);
    let logs = iso.drain_logs();
    let drained = iso.drain_interacts();
    iso.join();
    assert_eq!(ran, true, "logs={logs:?} interacts={drained:?}");
    assert!(
        drained.iter().any(|req| matches!(
            req,
            InteractReq::WalkNear {
                x: 2931,
                z: 9690,
                level: 0,
                radius: 1,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id,
            } if *request_id != 0
        )),
        "{drained:?}"
    );
    assert!(drained.iter().all(|req| match req {
        InteractReq::WalkTo { .. } => false,
        InteractReq::WalkNear {
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            radius,
            ..
        } => !allow_teleports && !allow_wilderness && !allow_bank_fetch && *radius == 1,
        InteractReq::Walk { .. } => false,
        _ => true,
    }));
}

// The function-style tasks are one-shot: the Rust row must not outlive the
// invocation, on the yield return included. The shim reads the binding off
// `rustyscript.functions` per call, so the isolate swaps the global for a
// spy proxy (same technique as the boost-potions bridge test) and then asks
// the real binding what the ended token is worth.
#[test]
fn cell_ends_its_rust_row_on_the_yield_return() {
    let src = r#"
import { cell } from '../../api/combat/hunting/combat.js';
export default class T extends LoopingBot {
    loop() {
        const site = {
            key: 'taverley-blue',
            keyItem: { id: 1590, name: 'Dusty key' },
            boxes: [{ minX: 40, maxX: 60, minZ: 40, maxZ: 60, level: 0 }],
        };
        if (!globalThis.__probe) {
            const original = globalThis.rustyscript;
            const ops = [];
            let begun = null;
            const spy = (payload) => {
                ops.push(payload && payload.op);
                const out = original.functions.__rs2b0t_cell(payload);
                if (payload && payload.op === 'begin') begun = out && out.token;
                return out;
            };
            let installed = false;
            try {
                globalThis.rustyscript = {
                    ...original,
                    functions: new Proxy({}, {
                        get(_target, name) {
                            return name === '__rs2b0t_cell' ? spy : original.functions[name];
                        },
                    }),
                };
                installed = globalThis.rustyscript !== original;
            } catch (e) {
                installed = false;
            }
            cell({ log() {}, setStatus() {} }, site);
            globalThis.rustyscript = original;
            globalThis.__probe = JSON.stringify({ installed, ops, begun });
            return;
        }
        const probe = JSON.parse(globalThis.__probe);
        const after = globalThis.rustyscript.functions.__rs2b0t_cell({
            op: 'next',
            token: probe.begun,
        });
        globalThis.__after = JSON.stringify({
            kind: after && after.kind,
            reason: after && after.reason,
        });
    }
}
"#;
    let dusty = ItemRowInput {
        name: Some("Dusty key"),
        count: 1,
        id: 1590,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot: 1,
    };
    let here = tile(2931, 9690, 0);
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    iso.post_snapshot(encode_snapshot(&snap_scene(here, &[dusty], &[], &[])));
    iso.on_game_tick(1);
    let probe: Value =
        serde_json::from_str(iso.probe("__probe").unwrap().as_str().unwrap()).unwrap();
    iso.post_snapshot(encode_snapshot(&snap_scene(here, &[dusty], &[], &[])));
    iso.on_game_tick(2);
    let after: Value =
        serde_json::from_str(iso.probe("__after").unwrap().as_str().unwrap()).unwrap();
    let logs = iso.drain_logs();
    iso.join();
    assert_eq!(probe["installed"], true, "the spy must be live: {probe}");
    assert_eq!(
        probe["ops"],
        json!(["begin", "next", "end"]),
        "the yield return ends the row: {probe} logs={logs:?}"
    );
    assert_ne!(probe["begun"], Value::Null, "{probe}");
    assert_eq!(after["kind"], "aborted", "{after} logs={logs:?}");
    assert_eq!(after["reason"], "unknown token", "{after}");
}

#[test]
fn v2_cell_next_keeps_abort_distinct_from_yield_false() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const began = api.cellBegin();
  globalThis.__begin = began;
  const token = began.value.token;
  const bad = api.cellNext({
    token: token + 1,
    key: 'heroes-blue',
    keyItem: null,
  });
  globalThis.__bad = bad;
  const aborted = api.cellNext({
    token,
    key: 'kbd-lair',
    keyItem: null,
  });
  globalThis.__aborted = aborted;
  const again = api.cellBegin();
  const done = api.cellNext({
    token: again.value.token,
    key: 'brimhaven-iron',
    keyItem: null,
  });
  globalThis.__done = done;
  const still = api.cellNext({
    token,
    key: 'heroes-blue',
    keyItem: null,
  });
  globalThis.__still = still;
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    iso.post_snapshot(encode_snapshot(&empty_snapshot(
        1,
        TileInput {
            x: 1,
            z: 1,
            level: 0,
        },
    )));
    iso.on_game_tick(1);
    let begin = iso.probe("globalThis.__begin").unwrap();
    assert_eq!(begin["ok"], true, "{begin:?}");
    let bad = iso.probe("globalThis.__bad").unwrap();
    assert_eq!(bad["ok"], false, "{bad:?}");
    assert_eq!(bad["kind"], "aborted");
    assert_eq!(bad["status"], "aborted");
    let aborted = iso.probe("globalThis.__aborted").unwrap();
    assert_eq!(aborted["ok"], false, "{aborted:?}");
    assert_eq!(aborted["error"], "kbd-later");
    assert_eq!(aborted["kind"], "aborted");
    assert_ne!(aborted["status"], "done");
    assert!(aborted.get("value").is_none());
    let done = iso.probe("globalThis.__done").unwrap();
    assert_eq!(done["ok"], true, "{done:?}");
    assert_eq!(done["status"], "done");
    assert_eq!(done["kind"], "yield");
    assert_eq!(done["value"], true);
    let still = iso.probe("globalThis.__still").unwrap();
    assert_eq!(still["ok"], false, "first token stays aborted: {still:?}");
    assert_eq!(still["kind"], "aborted");
    iso.join();
}

fn snap_rows<'a>(
    here: Tile,
    inv: &'a [ItemRowInput<'a>],
    bank: &'a [ItemRowInput<'a>],
) -> SnapshotInput<'a> {
    snap_scene(here, inv, &[], &[]).with_bank(bank)
}

fn snap_scene<'a>(
    here: Tile,
    inv: &'a [ItemRowInput<'a>],
    locs: &'a [SceneEntityInput<'a>],
    npcs: &'a [SceneEntityInput<'a>],
) -> SnapshotInput<'a> {
    let mut snap = empty_snapshot(
        1,
        TileInput {
            x: here.x,
            z: here.z,
            level: here.level,
        },
    );
    snap.inv = inv;
    snap.locs = locs;
    snap.npcs = npcs;
    snap
}

trait WithBank<'a> {
    fn with_bank(self, bank: &'a [ItemRowInput<'a>]) -> Self;
}

impl<'a> WithBank<'a> for SnapshotInput<'a> {
    fn with_bank(mut self, bank: &'a [ItemRowInput<'a>]) -> Self {
        self.bank = bank;
        self
    }
}

fn empty_snapshot(tick: u64, here: TileInput) -> SnapshotInput<'static> {
    SnapshotInput {
        tick,
        here: Some(here),
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
        reach: script::isolate_fb::ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
    }
}
