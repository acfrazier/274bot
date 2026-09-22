//! Offline AcquireKey isolate policy tests. No LIVE.
//!
//! These lock the architecture corrections: KBD/CELL abort, inv 1591, live
//! Chebyshev, index-not-name, one corridor approach, and walk-flag falses.
//! They do not mirror phases.

use script::hunt_fight::Tile;
use script::hunt_key::{
    self, key_deadline_remaining_ms, key_force_bound_reached, key_token_alive, set_observation,
    KeyGround, KeyInv, KeyNpc, KeyObservation,
};
use script::isolate_fb::{encode_snapshot, ItemRowInput, SnapshotInput, SnapshotReader, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use serde_json::{json, Value};

fn reset() {
    hunt_key::on_reset();
}

fn tile(x: i32, z: i32, level: i32) -> Tile {
    Tile { x, z, level }
}

fn outside() -> Tile {
    tile(2900, 9690, 0)
}

fn corridor() -> Tile {
    tile(2931, 9690, 0)
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

fn obs(here: Option<Tile>) -> KeyObservation {
    KeyObservation {
        here,
        ingame: true,
        hold: false,
        ours: false,
        inv: vec![],
        ground: vec![],
        npcs: vec![],
        locs: vec![],
    }
}

fn inv_row(id: i32, count: i32) -> KeyInv {
    KeyInv { id, count }
}

fn ground(id: i32, name: &str, at: Tile) -> KeyGround {
    KeyGround {
        id,
        name: name.to_string(),
        tile: at,
    }
}

fn jailer(index: i32, at: Tile) -> KeyNpc {
    KeyNpc {
        index,
        name: "Jailer".into(),
        actions: vec!["Attack".into()],
        tile: at,
    }
}

fn site(extra: Value) -> Value {
    let mut base = json!({
        "key": "taverley-blue",
        "keyItem": { "id": 1590 },
        "boxes": boxes(),
    });
    if let Some(obj) = extra.as_object() {
        for (k, v) in obj {
            base[k] = v.clone();
        }
    }
    base
}

fn heroes() -> Value {
    site(json!({ "key": "heroes-blue", "keyItem": null }))
}

fn begin() -> u64 {
    hunt_key::dispatch(&json!({ "op": "begin" }))["token"]
        .as_u64()
        .expect("token")
}

fn call(token: u64, mut p: Value, reply: Option<Value>) -> Value {
    p["op"] = json!("next");
    p["token"] = json!(token);
    if let Some(r) = reply {
        p["reply"] = r;
    }
    hunt_key::dispatch(&p)
}

fn walk_reply(token: u64) -> Value {
    json!({ "queued": true, "walkToken": token.wrapping_add(1000) })
}

fn kind(step: &Value) -> &str {
    step["kind"].as_str().unwrap_or("")
}

fn forbidden(step: &Value) -> bool {
    matches!(
        kind(step),
        "walk"
            | "walk-to"
            | "use-on"
            | "loc"
            | "bank-open"
            | "continue"
            | "answer"
            | "teleport"
            | "eat"
            | "arm-special"
    )
}

#[test]
fn heroes_unkeyed_yields_true_with_no_ops() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let step = call(token, heroes(), None);
    assert_eq!(kind(&step), "yield", "{step}");
    assert_eq!(step["value"], true);
    assert!(step.get("x").is_none());
    assert!(step.get("index").is_none());
    assert_ne!(step["kind"], "leave");
    assert_ne!(step["kind"], "npc");
    assert_ne!(step["kind"], "obj");
}

#[test]
fn kbd_key_aborts_before_unkeyed_yield() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let step = call(
        token,
        site(json!({ "key": "kbd-lair", "keyItem": null })),
        None,
    );
    assert_eq!(kind(&step), "aborted", "{step}");
    assert_eq!(step["reason"], "kbd-later");
    assert_ne!(kind(&step), "yield");
    assert!(step.get("value").is_none());
    assert!(step.get("index").is_none());
}

#[test]
fn projected_route_aborts_even_on_a_non_kbd_key() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let step = call(token, heroes(), None);
    assert_eq!(kind(&step), "yield");
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let step = call(
        token,
        site(json!({ "key": "heroes-blue", "keyItem": null, "route": {} })),
        None,
    );
    assert_eq!(kind(&step), "aborted", "{step}");
    assert_eq!(step["reason"], "kbd-later");
    assert_ne!(kind(&step), "yield");
}

#[test]
fn kbd_loc_ids_abort_and_a_jail_door_does_not() {
    for id in [1765, 1766, 1816, 1817] {
        reset();
        let mut seen = obs(Some(outside()));
        seen.locs = vec![id];
        set_observation(seen);
        let token = begin();
        let step = call(token, site(json!({})), None);
        assert_eq!(kind(&step), "aborted", "loc {id}: {step}");
        assert_eq!(step["reason"], "kbd-later");
        assert_ne!(kind(&step), "yield");
        assert!(step.get("index").is_none(), "no Jailer attack on KBD");
    }
    reset();
    let mut seen = obs(Some(outside()));
    seen.locs = vec![2631];
    set_observation(seen);
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_ne!(step["reason"], "kbd-later", "{step}");
    assert_ne!(kind(&step), "loc");
    assert_ne!(step["z"], 9689);
}

#[test]
fn cell_aborts_even_when_inv_already_holds_the_key() {
    reset();
    let mut seen = obs(Some(cell_here()));
    seen.inv = vec![inv_row(1591, 1)];
    set_observation(seen);
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_eq!(kind(&step), "aborted", "{step}");
    assert_eq!(step["reason"], "cell-later");
    assert_ne!(kind(&step), "yield");
    assert_ne!(step["value"], true);
    assert!(step.get("x").is_none());
    assert_ne!(kind(&step), "loc");
}

#[test]
fn cell_fence_is_not_site_boxes_and_stops_at_the_door_tile() {
    reset();
    set_observation(obs(Some(tile(2931, 9689, 0))));
    let token = begin();
    let step = call(token, site(json!({ "boxes": [] })), None);
    assert_eq!(step["reason"], "cell-later", "{step}");
    assert_ne!(kind(&step), "yield");

    reset();
    set_observation(obs(Some(tile(2931, 9690, 0))));
    let token = begin();
    let step = call(token, site(json!({ "boxes": [] })), None);
    assert_ne!(step["reason"], "cell-later", "{step}");
    assert_ne!(step["z"], 9689);

    reset();
    set_observation(obs(Some(tile(2931, 9686, 1))));
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_ne!(
        step["reason"], "cell-later",
        "other level is not the cell: {step}"
    );
}

#[test]
fn already_held_yields_true_outside_the_cell_and_does_not_leave() {
    reset();
    let mut seen = obs(Some(lair_here()));
    seen.inv = vec![inv_row(1591, 1)];
    set_observation(seen);
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_eq!(kind(&step), "yield", "{step}");
    assert_eq!(step["value"], true);
    assert_ne!(kind(&step), "leave");
    assert!(step.get("index").is_none());
}

#[test]
fn dusty_key_and_zero_count_do_not_complete() {
    reset();
    let mut seen = obs(Some(outside()));
    seen.inv = vec![inv_row(1590, 5), inv_row(1591, 0)];
    set_observation(seen);
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_ne!(kind(&step), "yield", "{step}");
    assert_ne!(step["value"], true);
}

#[test]
fn key_item_id_does_not_complete() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let step = call(token, site(json!({ "keyItem": { "id": 1591 } })), None);
    assert_ne!(step["value"], true, "{step}");
    assert_ne!(kind(&step), "yield");
}

#[test]
fn hold_does_not_claim_the_key() {
    reset();
    let mut seen = obs(Some(outside()));
    seen.inv = vec![inv_row(1591, 3)];
    seen.hold = true;
    set_observation(seen);
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_eq!(kind(&step), "yield", "{step}");
    assert_eq!(step["value"], false);
}

#[test]
fn still_inside_emits_leave_before_take_or_attack() {
    reset();
    let mut seen = obs(Some(lair_here()));
    seen.ground = vec![ground(1591, "Jail key", tile(50, 51, 0))];
    seen.npcs = vec![jailer(7, tile(50, 52, 0))];
    set_observation(seen);
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_eq!(kind(&step), "leave", "{step}");
    assert_ne!(kind(&step), "npc");
    assert_ne!(kind(&step), "obj");
    assert_ne!(kind(&step), "walk-near");
    let failed = call(token, site(json!({})), Some(json!({ "left": false })));
    assert_eq!(kind(&failed), "yield", "{failed}");
    assert_eq!(failed["value"], false);
}

#[test]
fn leave_true_does_not_prove_the_key() {
    reset();
    set_observation(obs(Some(lair_here())));
    let token = begin();
    assert_eq!(kind(&call(token, site(json!({})), None)), "leave");
    set_observation(obs(Some(outside())));
    let step = call(token, site(json!({})), Some(json!({ "left": true })));
    assert_ne!(step["value"], true, "{step}");
    assert_eq!(kind(&step), "walk-near", "{step}");
    assert_eq!(step["x"], 2931);
    assert_eq!(step["z"], 9690);
    assert_ne!(step["z"], 9689);
}

#[test]
fn lair_membership_uses_size_one() {
    reset();
    set_observation(obs(Some(tile(10, 10, 0))));
    let token = begin();
    let step = call(
        token,
        site(json!({
            "boxes": [{ "minX": 11, "maxX": 12, "minZ": 10, "maxZ": 10, "level": 0 }]
        })),
        None,
    );
    assert_ne!(
        kind(&step),
        "leave",
        "size 1 must not treat a body-origin box as inside: {step}"
    );
    assert_eq!(kind(&step), "walk-near");
    assert_eq!(step["x"], 2931);
    assert_eq!(step["z"], 9690);
}

#[test]
fn drop_within_twelve_is_taken_before_corridor_or_attack() {
    reset();
    let mut seen = obs(Some(outside()));
    seen.ground = vec![ground(1591, "Jail key", tile(2905, 9690, 0))];
    seen.npcs = vec![jailer(7, tile(2902, 9690, 0))];
    set_observation(seen);
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_eq!(kind(&step), "walk-near", "{step}");
    assert_eq!(step["x"], 2905);
    assert_eq!(step["z"], 9690);
    assert_eq!(step["radius"], 1);
    assert_ne!(
        step["x"], 2931,
        "do not walk the corridor away from a visible drop"
    );
    assert_ne!(kind(&step), "npc");
}

#[test]
fn live_chebyshev_ignores_a_far_tile_and_keeps_a_near_one() {
    reset();
    let mut far = obs(Some(outside()));
    far.ground = vec![ground(1591, "Jail key", tile(2920, 9690, 0))];
    set_observation(far);
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_ne!(step["x"], 2920, "live 20 is outside 12: {step}");
    assert_eq!(kind(&step), "walk-near");
    assert_eq!(step["x"], 2931);
    assert_eq!(step["z"], 9690);

    reset();
    let mut near = obs(Some(outside()));
    near.ground = vec![
        ground(1590, "Dusty key", tile(2900, 9690, 0)),
        ground(1591, "Jail key", tile(2908, 9693, 0)),
        ground(1591, "Jail key", tile(2903, 9690, 0)),
    ];
    set_observation(near);
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_eq!(kind(&step), "walk-near", "{step}");
    assert_eq!(
        step["x"], 2903,
        "nearest live drop, not 1590 and not the farther 1591"
    );
    assert_eq!(step["z"], 9690);
    assert_eq!(step["radius"], 1);
}

#[test]
fn other_level_drop_is_not_within_twelve() {
    reset();
    let mut seen = obs(Some(outside()));
    seen.ground = vec![ground(1591, "Jail key", tile(2900, 9690, 1))];
    set_observation(seen);
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_ne!(kind(&step), "obj", "{step}");
    assert_ne!(step["level"], 1);
    assert_eq!(step["z"], 9690);
    assert_eq!(step["x"], 2931);
}

#[test]
fn distance_one_takes_without_a_walk_and_arrival_is_not_the_key() {
    reset();
    let mut seen = obs(Some(tile(2900, 9690, 0)));
    seen.ground = vec![ground(1591, "Jail key", tile(2901, 9690, 0))];
    set_observation(seen);
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_eq!(kind(&step), "obj", "{step}");
    assert_eq!(step["action"], "Take");
    assert_eq!(step["name"], "Jail key");
    assert_eq!(step["x"], 2901);
    assert!(step.get("id").is_none(), "obj carries no item id");
    let queued = call(token, site(json!({})), Some(json!({ "queued": true })));
    assert_ne!(kind(&queued), "yield", "queued is not the key: {queued}");
    assert_ne!(queued["value"], true);
    set_observation(obs(Some(tile(2901, 9690, 0))));
    let still = call(token, site(json!({})), None);
    assert_ne!(
        still["value"], true,
        "standing on the tile is not the key: {still}"
    );
}

#[test]
fn nameless_drop_emits_no_obj() {
    reset();
    let mut seen = obs(Some(outside()));
    seen.ground = vec![ground(1591, "", tile(2900, 9690, 0))];
    set_observation(seen);
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_ne!(kind(&step), "obj", "{step}");
    assert_eq!(kind(&step), "yield");
    assert_eq!(step["value"], false);
}

#[test]
fn take_proof_retries_then_yields_false() {
    reset();
    let mut seen = obs(Some(tile(2900, 9690, 0)));
    seen.ground = vec![ground(1591, "Jail key", tile(2900, 9690, 0))];
    set_observation(seen);
    let token = begin();
    let p = site(json!({}));
    for n in 1..=3 {
        let step = call(token, p.clone(), Some(json!({ "queued": true })));
        assert_eq!(kind(&step), "obj", "attempt {n}: {step}");
        assert!(key_force_bound_reached(token));
    }
    let done = call(token, p, Some(json!({ "queued": true })));
    assert_eq!(kind(&done), "yield", "{done}");
    assert_eq!(done["value"], false);
}

#[test]
fn pickup_window_is_eight_seconds_not_the_fight_bound() {
    reset();
    let mut seen = obs(Some(tile(2900, 9690, 0)));
    seen.ground = vec![ground(1591, "Jail key", tile(2900, 9690, 0))];
    set_observation(seen);
    let token = begin();
    assert_eq!(kind(&call(token, site(json!({})), None)), "obj");
    let left = key_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        left > 7_000 && left <= 8_000,
        "pickup window must be 8000, got {left}"
    );
    assert!(left < 90_000);
}

#[test]
fn jailer_attack_uses_that_rows_index_not_the_name_alone() {
    reset();
    let mut seen = obs(Some(outside()));
    seen.npcs = vec![
        KeyNpc {
            index: 3,
            name: "Jailer".into(),
            actions: vec!["Attack".into()],
            tile: tile(2912, 9690, 0),
        },
        jailer(7, tile(2904, 9690, 0)),
        KeyNpc {
            index: 9,
            name: "Jailer".into(),
            actions: vec!["Talk-to".into()],
            tile: tile(2901, 9690, 0),
        },
        KeyNpc {
            index: 4,
            name: "Goblin".into(),
            actions: vec!["Attack".into()],
            tile: tile(2900, 9690, 0),
        },
    ];
    set_observation(seen);
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_eq!(kind(&step), "npc", "{step}");
    assert_eq!(step["name"], "Jailer");
    assert_eq!(step["action"], "Attack");
    assert_eq!(step["index"], 7);
    assert_ne!(step["action"], "Fight");
    assert_ne!(kind(&step), "eat");
    assert_ne!(kind(&step), "arm-special");
    let again = call(token, site(json!({})), Some(json!({ "queued": true })));
    assert_ne!(kind(&again), "npc", "do not keep swinging: {again}");
}

#[test]
fn reused_index_is_death_then_take_not_another_attack() {
    reset();
    let mut seen = obs(Some(corridor()));
    seen.npcs = vec![jailer(7, tile(2932, 9690, 0))];
    set_observation(seen);
    let token = begin();
    let p = site(json!({}));
    let swing = call(token, p.clone(), None);
    assert_eq!(swing["index"], 7, "{swing}");
    let mut dead = obs(Some(corridor()));
    dead.npcs = vec![KeyNpc {
        index: 7,
        name: "Goblin".into(),
        actions: vec!["Attack".into()],
        tile: tile(2932, 9690, 0),
    }];
    dead.ground = vec![ground(1591, "Jail key", tile(2931, 9690, 0))];
    set_observation(dead);
    let step = call(token, p, Some(json!({ "queued": true })));
    assert_ne!(kind(&step), "npc", "{step}");
    assert_ne!(step["value"], true);
    assert_eq!(kind(&step), "obj");
    assert_eq!(step["name"], "Jail key");
    assert_eq!(step["action"], "Take");
}

#[test]
fn attack_window_is_ninety_seconds() {
    reset();
    let mut seen = obs(Some(outside()));
    seen.npcs = vec![jailer(7, tile(2904, 9690, 0))];
    set_observation(seen);
    let token = begin();
    assert_eq!(kind(&call(token, site(json!({})), None)), "npc");
    let left = key_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        left > 80_000 && left <= 90_000,
        "attack window must be 90000, not 120000, got {left}"
    );
}

#[test]
fn live_jailer_past_the_window_yields_false() {
    reset();
    let mut seen = obs(Some(outside()));
    seen.npcs = vec![jailer(7, tile(2904, 9690, 0))];
    set_observation(seen);
    let token = begin();
    let p = site(json!({}));
    assert_eq!(kind(&call(token, p.clone(), None)), "npc");
    assert!(key_force_bound_reached(token));
    let step = call(token, p, Some(json!({ "queued": true })));
    assert_eq!(kind(&step), "yield", "{step}");
    assert_eq!(step["value"], false);
}

#[test]
fn blind_corridor_is_one_approach_then_respawn_wait() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let p = site(json!({}));
    let step = call(token, p.clone(), None);
    assert_eq!(kind(&step), "walk-near", "{step}");
    assert_eq!(step["x"], 2931);
    assert_eq!(step["z"], 9690);
    assert_eq!(step["level"], 0);
    assert_eq!(step["radius"], 1);
    assert_ne!(step["z"], 9689);
    assert_ne!(kind(&step), "walk");
    let left = key_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        left > 200_000 && left <= 300_000,
        "walk leg must be 300000, got {left}"
    );

    set_observation(obs(Some(corridor())));
    let arrived = call(token, p.clone(), Some(walk_reply(token)));
    assert_ne!(
        kind(&arrived),
        "walk-near",
        "one approach, not a loop: {arrived}"
    );
    assert_eq!(kind(&arrived), "sustain", "{arrived}");
    let respawn = key_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        respawn > 70_000 && respawn <= 75_000,
        "respawn window must be 75000, got {respawn}"
    );

    let mut seen = obs(Some(corridor()));
    seen.npcs = vec![jailer(4, tile(2932, 9690, 0))];
    set_observation(seen);
    let swing = call(token, p.clone(), None);
    assert_eq!(kind(&swing), "npc", "{swing}");
    set_observation(obs(Some(corridor())));
    let after = call(token, p, Some(json!({ "queued": true })));
    assert_ne!(
        kind(&after),
        "walk-near",
        "do not walk the corridor again: {after}"
    );
    assert_ne!(after["value"], true);
    assert_eq!(kind(&after), "sustain");
}

#[test]
fn already_within_radius_one_respawn_waits_without_a_walk() {
    reset();
    set_observation(obs(Some(tile(2932, 9690, 0))));
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_ne!(kind(&step), "walk-near", "{step}");
    assert_eq!(kind(&step), "sustain");
    assert_ne!(step["z"], 9689);
}

#[test]
fn failed_corridor_walk_yields_false() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let p = site(json!({}));
    assert_eq!(kind(&call(token, p.clone(), None)), "walk-near");
    assert!(key_force_bound_reached(token));
    let step = call(token, p, Some(walk_reply(token)));
    assert_eq!(kind(&step), "yield", "{step}");
    assert_eq!(step["value"], false);
    assert_ne!(kind(&step), "sustain");
}

#[test]
fn respawn_window_elapses_false_and_a_jailer_interrupts_it() {
    reset();
    set_observation(obs(Some(corridor())));
    let token = begin();
    let p = site(json!({}));
    assert_eq!(kind(&call(token, p.clone(), None)), "sustain");
    assert!(key_force_bound_reached(token));
    let done = call(token, p.clone(), None);
    assert_eq!(kind(&done), "yield");
    assert_eq!(done["value"], false);

    reset();
    set_observation(obs(Some(corridor())));
    let token = begin();
    assert_eq!(kind(&call(token, p.clone(), None)), "sustain");
    let mut seen = obs(Some(corridor()));
    seen.npcs = vec![jailer(11, tile(2931, 9691, 0))];
    set_observation(seen);
    let step = call(token, p, None);
    assert_eq!(kind(&step), "npc", "{step}");
    assert_eq!(step["index"], 11);
}

#[test]
fn gone_drop_after_the_walk_is_not_taken() {
    reset();
    let mut seen = obs(Some(outside()));
    seen.ground = vec![ground(1591, "Jail key", tile(2908, 9690, 0))];
    set_observation(seen);
    let token = begin();
    let p = site(json!({}));
    let walk = call(token, p.clone(), None);
    assert_eq!(walk["x"], 2908, "{walk}");
    set_observation(obs(Some(tile(2908, 9690, 0))));
    let step = call(token, p, Some(walk_reply(token)));
    assert_ne!(kind(&step), "obj", "do not Take the pre-walk tile: {step}");
    assert_ne!(step["value"], true);
}

#[test]
fn ours_mid_wait_yields_false() {
    reset();
    set_observation(obs(Some(corridor())));
    let token = begin();
    let p = site(json!({}));
    assert_eq!(kind(&call(token, p.clone(), None)), "sustain");
    let mut seen = obs(Some(corridor()));
    seen.ours = true;
    seen.inv = vec![inv_row(1591, 1)];
    set_observation(seen);
    let step = call(token, p, None);
    assert_eq!(kind(&step), "yield", "{step}");
    assert_eq!(step["value"], false);
}

#[test]
fn key_pause_freezes_the_clock_and_fight_pause_does_not() {
    reset();
    let mut seen = obs(Some(outside()));
    seen.npcs = vec![jailer(7, tile(2904, 9690, 0))];
    set_observation(seen);
    let token = begin();
    let p = site(json!({}));
    assert_eq!(kind(&call(token, p.clone(), None)), "npc");
    let before = key_deadline_remaining_ms(token).unwrap_or(0);
    script::hunt_fight::on_pause();
    script::hunt_leave::on_pause();
    std::thread::sleep(std::time::Duration::from_millis(220));
    let stepped = call(token, p.clone(), Some(json!({ "queued": true })));
    assert_ne!(
        kind(&stepped),
        "wait",
        "Fight/Leave pause must not freeze key"
    );
    let mid = key_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        before - mid > 80,
        "key clock elapsed under Fight pause: {before} -> {mid}"
    );
    hunt_key::on_pause();
    std::thread::sleep(std::time::Duration::from_millis(280));
    let frozen = call(token, p, None);
    assert_eq!(kind(&frozen), "wait");
    hunt_key::on_resume();
    let after = key_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        (mid - after).abs() < 80,
        "window elapsed during key pause: {mid} -> {after}"
    );
    script::hunt_fight::on_resume();
    script::hunt_leave::on_resume();
}

#[test]
fn unexpected_reply_and_wrong_token_abort() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let eat = call(token, site(json!({})), Some(json!({ "eatOk": true })));
    assert_eq!(kind(&eat), "aborted");
    let bad = call(token + 9, site(json!({})), None);
    assert_eq!(kind(&bad), "aborted");
    assert_eq!(bad["reason"], "unknown token");
    assert!(key_token_alive(token));
    hunt_key::on_reset();
    assert!(!key_token_alive(token));
}

#[test]
fn snapshot_reads_inv_id_not_cert_bank_or_equipment() {
    reset();
    let jail = ItemRowInput {
        name: Some("Jail key"),
        count: 1,
        id: 1591,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot: 0,
    };
    let cert = ItemRowInput {
        name: Some("Jail key"),
        count: 5,
        id: 9999,
        ops: &[],
        noted: true,
        cert: 1591,
        component_id: -1,
        slot: 1,
    };
    let dusty = ItemRowInput {
        name: Some("Dusty key"),
        count: 1,
        id: 1590,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot: 2,
    };
    let bytes = encode_snapshot(&snap_rows(outside(), &[cert, dusty], &[jail], &[jail]));
    let reader = SnapshotReader::from_bytes(&bytes).unwrap();
    hunt_key::on_snapshot(&reader);
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_ne!(
        step["value"], true,
        "cert, bank, and 1590 are not the key: {step}"
    );

    reset();
    let bytes = encode_snapshot(&snap_rows(outside(), &[jail], &[], &[]));
    let reader = SnapshotReader::from_bytes(&bytes).unwrap();
    hunt_key::on_snapshot(&reader);
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert_eq!(kind(&step), "yield", "{step}");
    assert_eq!(step["value"], true);
}

#[test]
fn key_effects_never_include_forbidden_ops() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let step = call(token, site(json!({})), None);
    assert!(!forbidden(&step), "{step}");
    let src = include_str!("../src/hunt_key.rs");
    assert!(!src.contains("hunt_leave::dispatch"));
    assert!(!src.contains("hunt_fight::dispatch"));
    assert!(!src.contains("gap_sw"));
    assert!(!src.contains("bodyOrigin"));
    assert!(!src.contains("size>>1"));
    assert!(!src.contains("size >> 1"));
    assert!(!src.contains("FIGHT_MS"));
    assert!(!src.contains("3094"));
    assert!(!src.contains("varrock"));
    assert!(!src.contains("keyStatus"));
    assert!(!src.contains(".bank("));
    assert!(!src.contains("equipment"));
    assert!(!src.contains(".distance()"));
    assert!(!src.contains(".nx()"));
    assert!(!src.contains(".nz()"));
    assert!(src.contains("in_area_body(here, 1,"));
    assert!(src.contains("KEY_RUNTIMES"));
}

#[test]
fn shim_and_bindings_keep_the_yield_shape_and_flag_falses() {
    let bindings = include_str!("../src/load/bindings.rs");
    let key = bindings.split("api.keyNext").nth(1).expect("keyNext");
    let key = key.split("function recordSettlement").next().unwrap();
    assert!(key.contains("kind: 'yield', value:"));
    assert!(key.contains("ok: false, error:"));
    assert!(key.contains("kind: 'aborted'"));
    assert!(key.contains("status: 'aborted'"));
    assert!(!key.contains("ok: true, status: 'aborted'"));
    assert!(!bindings.contains("api.keyValidate"));
    let reg = bindings.split("__rs2b0t_key").nth(1).unwrap();
    let reg = reg.split("__rs2b0t_production").next().unwrap();
    assert!(!reg.contains("SelectedGameData"));
    assert!(!reg.contains("selected_"));

    let dts = include_str!("../src/host_js.rs");
    let step = dts.split("export type KeyStep").nth(1).expect("KeyStep");
    assert!(step.contains("kind: 'yield'; value: boolean"));
    assert!(step.contains("ok: false; error: string; kind: 'aborted'"));
    assert!(!dts.contains("keyValidate"));

    let js = include_str!("../src/shim/hunting_combat.js");
    let acquire = js
        .split("export async function acquireKey")
        .nth(1)
        .expect("acquireKey");
    assert!(acquire.contains("leaveLair"));
    assert!(acquire.contains("allow_teleports: false"));
    assert!(acquire.contains("allow_wilderness: false"));
    assert!(acquire.contains("allow_bank_fetch: false"));
    assert!(!acquire.contains("1591"));
    assert!(!acquire.contains("Jailer"));
    assert!(!acquire.contains("walk-to"));
    assert!(!acquire.contains("case 'loc'"));
    assert!(!acquire.contains("use-on"));
    assert!(!acquire.contains("inArea"));
    assert!(!acquire.contains("host.fight"));
    let begin = js.split("function beginKeyWalk").nth(1).unwrap();
    let begin = begin
        .split("export async function acquireKey")
        .next()
        .unwrap();
    assert!(begin.contains("radius,"));
    assert!(!begin.contains("radius: 0"));
    assert!(begin.contains("allow_wilderness: false"));
    assert!(!begin.contains("allow_wilderness: true"));

    let iso = include_str!("../src/load/isolate.rs");
    for hook in [
        "on_snapshot",
        "on_pause",
        "on_hold",
        "on_resume",
        "on_reset",
    ] {
        assert!(iso.contains(&format!("hunt_key::{hook}")), "missing {hook}");
        assert!(iso.contains(&format!("hunt_leave::{hook}")));
    }
}

#[test]
fn queued_key_walk_flags_are_false_and_radius_is_one() {
    let src = r#"
import { acquireKey } from '../../api/combat/hunting/combat.js';
export default class T extends LoopingBot {
    loop() {
        const host = {
            fight: { id: 7 },
            log() {},
            setStatus() {},
        };
        const site = {
            key: 'taverley-blue',
            keyItem: { id: 1590 },
            boxes: [{ minX: 40, maxX: 60, minZ: 40, maxZ: 60, level: 0 }],
        };
        acquireKey(host, site);
        globalThis.__fight = host.fight.id;
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    iso.post_snapshot(encode_snapshot(&empty_snapshot(
        1,
        TileInput {
            x: 1,
            z: 1,
            level: 0,
        },
    )));
    iso.on_game_tick(1);
    let fight = iso.probe("globalThis.__fight").unwrap_or(Value::Null);
    let drained = iso.drain_interacts();
    iso.join();
    assert_eq!(fight, 7);
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
        InteractReq::Walk { .. } | InteractReq::WalkTo { .. } | InteractReq::Loc { .. } => false,
        InteractReq::WalkNear {
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            radius,
            ..
        } => !allow_teleports && !allow_wilderness && !allow_bank_fetch && *radius == 1,
        _ => true,
    }));
}

#[test]
fn v2_key_next_keeps_abort_distinct_from_yield_false() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const began = api.keyBegin();
  globalThis.__begin = began;
  const token = began.value.token;
  const bad = api.keyNext({
    token: token + 1,
    key: 'heroes-blue',
    keyItem: null,
  });
  globalThis.__bad = bad;
  const aborted = api.keyNext({
    token,
    key: 'kbd-lair',
    keyItem: null,
  });
  globalThis.__aborted = aborted;
  const again = api.keyBegin();
  const done = api.keyNext({
    token: again.value.token,
    key: 'heroes-blue',
    keyItem: null,
  });
  globalThis.__done = done;
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
    assert_ne!(bad["status"], "done");
    let aborted = iso.probe("globalThis.__aborted").unwrap();
    assert_eq!(aborted["ok"], false, "{aborted:?}");
    assert_eq!(aborted["kind"], "aborted");
    assert_eq!(aborted["status"], "aborted");
    assert_eq!(aborted["error"], "kbd-later");
    assert_ne!(aborted["status"], "done");
    let done = iso.probe("globalThis.__done").unwrap();
    assert_eq!(done["ok"], true, "{done:?}");
    assert_eq!(done["status"], "done");
    assert_eq!(done["kind"], "yield");
    assert_eq!(done["value"], true);
    iso.join();
}

fn snap_rows<'a>(
    here: Tile,
    inv: &'a [ItemRowInput<'a>],
    bank: &'a [ItemRowInput<'a>],
    equipment: &'a [ItemRowInput<'a>],
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
    snap.bank = bank;
    snap.equipment = equipment;
    snap
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
