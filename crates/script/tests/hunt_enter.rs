//! Offline EnterLair isolate policy tests. No LIVE.

use script::hunt_fight::{self, Tile};
use script::hunt_lair::{
    self, enter_deadline_remaining_ms, enter_force_bound_reached, enter_token_alive, fee_paid_for,
    set_observation, EnterChatLine, EnterInv, EnterLoc, EnterNpc, EnterObservation,
};
use script::isolate_fb::{SnapshotInput, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use serde_json::{json, Value};

fn reset() {
    hunt_lair::on_reset();
}

fn tile(x: i32, z: i32, level: i32) -> Tile {
    Tile { x, z, level }
}

fn outside() -> Tile {
    tile(1, 1, 0)
}

fn inside() -> Tile {
    tile(50, 50, 0)
}

fn lair_box() -> Value {
    json!([{ "minX": 40, "maxX": 60, "minZ": 40, "maxZ": 60, "level": 0 }])
}

fn ready() -> Value {
    json!({
        "parked": false,
        "shieldReady": true,
        "hpFraction": 1.0,
        "panicHp": 0.2,
    })
}

fn with_site(mut base: Value, site: Value) -> Value {
    let obj = base.as_object_mut().unwrap();
    for (k, v) in site.as_object().unwrap() {
        obj.insert(k.clone(), v.clone());
    }
    base
}

fn talk_site() -> Value {
    with_site(
        ready(),
        json!({
            "key": "gutanoth-blue",
            "boxes": lair_box(),
            "approach": [],
            "talkGate": {
                "npc": "Guard",
                "op": "Talk-to",
                "choose": "I want to go in there",
                "stand": { "x": 10, "z": 10, "level": 0 }
            },
            "feeGate": null,
            "gate": {
                "locId": 2623,
                "op": "Open",
                "outside": { "x": 20, "z": 20, "level": 0 }
            },
            "keyItem": null,
        }),
    )
}

fn fee_site() -> Value {
    with_site(
        ready(),
        json!({
            "key": "brimhaven-iron",
            "boxes": lair_box(),
            "approach": [],
            "talkGate": null,
            "feeGate": {
                "npc": "Saniboch",
                "op": "Pay",
                "coins": 875,
                "stand": { "x": 12, "z": 12, "level": 0 },
                "entrance": { "locId": 5083, "op": "Enter" },
                "paidLine": "you pay saniboch 875 coins",
                "prepaidLine": "already given me lots of nice coins"
            },
            "gate": null,
            "keyItem": null,
        }),
    )
}

fn gate_site(key_item: Value) -> Value {
    with_site(
        ready(),
        json!({
            "key": "taverley-blue",
            "boxes": lair_box(),
            "approach": [],
            "talkGate": null,
            "feeGate": null,
            "gate": {
                "locId": 2623,
                "op": "Open",
                "outside": { "x": 20, "z": 20, "level": 0 }
            },
            "keyItem": key_item,
        }),
    )
}

fn gateless_site(approach: Value) -> Value {
    with_site(
        ready(),
        json!({
            "key": "heroes-blue",
            "boxes": lair_box(),
            "approach": approach,
            "talkGate": null,
            "feeGate": null,
            "gate": null,
            "keyItem": null,
        }),
    )
}

fn obs(here: Option<Tile>) -> EnterObservation {
    EnterObservation {
        here,
        ingame: true,
        hold: false,
        ours: false,
        chat_continue: false,
        chat_open: false,
        chat_lines: vec![],
        chat_options: vec![],
        main_modal_id: -1,
        npcs: vec![],
        locs: vec![],
        inv: vec![],
        tick: 1,
    }
}

fn begin() -> u64 {
    hunt_lair::dispatch(&json!({ "op": "begin" }))["token"]
        .as_u64()
        .expect("token")
}

fn call(op: &str, token: u64, p: Value, reply: Option<Value>) -> Value {
    let mut v = p;
    v["op"] = json!(op);
    v["token"] = json!(token);
    if let Some(r) = reply {
        v["reply"] = r;
    }
    hunt_lair::dispatch(&v)
}

fn validate(token: u64, p: Value) -> bool {
    call("validate", token, p, None)["value"]
        .as_bool()
        .unwrap_or(false)
}

fn ack_walk(token: u64) -> Value {
    json!({ "queued": true, "walkToken": token.wrapping_add(1000) })
}

fn kinds_until(token: u64, p: Value, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut reply = None;
    for _ in 0..limit {
        let step = call("next", token, p.clone(), reply.take());
        let kind = step["kind"].as_str().unwrap_or("").to_string();
        out.push(kind.clone());
        if kind == "yield" || kind == "aborted" {
            break;
        }
        if kind == "walk"
            || kind == "walk-near"
            || kind == "npc"
            || kind == "loc"
            || kind == "use-on"
        {
            reply = Some(json!({ "queued": true, "walkToken": token.wrapping_add(1000) }));
        }
    }
    out
}

#[test]
fn already_in_area_validate_false_execute_true_with_no_ops() {
    reset();
    set_observation(obs(Some(inside())));
    let token = begin();
    let p = gateless_site(json!([]));
    assert!(
        !validate(token, p.clone()),
        "already inArea is not a validate pass"
    );
    let step = call("next", token, p, None);
    assert_eq!(step["kind"], "yield");
    assert_eq!(step["value"], true);
    assert_ne!(step["kind"], "walk");
    assert_ne!(step["kind"], "npc");
    assert_ne!(step["kind"], "loc");
}

#[test]
fn validate_skips_parked_unready_panic_and_in_area() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let base = gateless_site(json!([]));
    assert!(!validate(
        token,
        with_site(base.clone(), json!({ "parked": true }))
    ));
    assert!(!validate(
        token,
        with_site(base.clone(), json!({ "shieldReady": false }))
    ));
    assert!(!validate(
        token,
        with_site(base.clone(), json!({ "hpFraction": 0.1 }))
    ));
    set_observation(obs(Some(inside())));
    assert!(!validate(token, base));
}

#[test]
fn short_coins_are_false_in_validate_and_execute_without_bank_open() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let p = fee_site();
    assert!(!validate(token, p.clone()));
    let mut saw_bank = false;
    let mut saw_log = false;
    let mut reply = None;
    let mut yielded = None;
    for _ in 0..8 {
        let step = call("next", token, p.clone(), reply.take());
        let kind = step["kind"].as_str().unwrap_or("");
        saw_bank |= kind == "bank-open";
        if kind == "log" {
            saw_log |= step["message"]
                .as_str()
                .unwrap_or("")
                .contains("Banking for more");
        }
        if kind == "yield" {
            yielded = Some(step["value"].as_bool().unwrap_or(true));
            break;
        }
    }
    assert!(saw_log, "short coins logs the banking line");
    assert!(!saw_bank);
    assert_eq!(yielded, Some(false));
}

#[test]
fn missing_key_validate_false() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let p = gate_site(json!({ "name": "Dusty key", "id": 1590 }));
    assert!(!validate(token, p));
}

#[test]
fn talk_gate_wins_and_stands_are_walk_near_radius_2() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let p = talk_site();
    let status = call("next", token, p.clone(), None);
    assert_eq!(status["kind"], "status");
    let step = call("next", token, p, None);
    assert_eq!(step["kind"], "walk-near");
    assert_eq!(step["radius"], 2);
    assert_eq!(step["x"], 10);
    assert_eq!(step["z"], 10);
    assert_ne!(step["kind"], "walk");
    assert_ne!(step["kind"], "walk-to");
    assert_ne!(step["kind"], "loc");
}

#[test]
fn choose_hit_answers_the_fragment_index_not_the_last() {
    reset();
    let mut seen = obs(Some(tile(10, 10, 0)));
    seen.npcs = vec![EnterNpc {
        index: 4,
        name: "Guard".into(),
        actions: vec!["Talk-to".into()],
        distance: 1,
    }];
    seen.chat_options = vec![
        "Go away".into(),
        "I want to go in there".into(),
        "Never mind".into(),
    ];
    set_observation(seen);
    let token = begin();
    let p = talk_site();
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk-near");
    let npc = call("next", token, p.clone(), Some(ack_walk(token)));
    assert_eq!(npc["kind"], "npc");
    assert_eq!(npc["action"], "Talk-to");
    let answer = call("next", token, p, Some(json!({ "queued": true })));
    assert_eq!(answer["kind"], "answer");
    assert_eq!(
        answer["option"], 2,
        "1-based fragment index, not the last option"
    );
}

#[test]
fn fragment_miss_logs_watch_tower_and_emits_no_answer() {
    reset();
    let mut seen = obs(Some(tile(10, 10, 0)));
    seen.npcs = vec![EnterNpc {
        index: 4,
        name: "Guard".into(),
        actions: vec!["Talk-to".into()],
        distance: 1,
    }];
    seen.chat_options = vec!["Yes".into(), "No".into()];
    set_observation(seen);
    let token = begin();
    let p = talk_site();
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk-near");
    assert_eq!(
        call("next", token, p.clone(), Some(ack_walk(token)))["kind"],
        "npc"
    );
    let miss = call("next", token, p.clone(), Some(json!({ "queued": true })));
    assert_eq!(miss["kind"], "log");
    assert!(
        miss["message"]
            .as_str()
            .unwrap_or("")
            .contains("Watch Tower"),
        "{miss}"
    );
    let mut saw_answer = false;
    let mut value = None;
    for _ in 0..4 {
        let step = call("next", token, p.clone(), None);
        saw_answer |= step["kind"] == "answer";
        if step["kind"] == "yield" {
            value = step["value"].as_bool();
            break;
        }
    }
    assert!(!saw_answer, "fragment miss emits no answer");
    assert_eq!(value, Some(false));
}

#[test]
fn fee_proof_is_not_the_walk_and_sets_prepaid_before_loc() {
    reset();
    let mut seen = obs(Some(tile(12, 12, 0)));
    seen.npcs = vec![EnterNpc {
        index: 3,
        name: "Saniboch".into(),
        actions: vec!["Pay".into()],
        distance: 1,
    }];
    seen.inv = vec![EnterInv {
        id: 995,
        count: 1000,
        slot: 0,
        name: "Coins".into(),
    }];
    set_observation(seen.clone());
    let token = begin();
    let p = fee_site();
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk-near");
    let paying = call("next", token, p.clone(), Some(ack_walk(token)));
    assert_eq!(paying["kind"], "status");
    let npc = call("next", token, p.clone(), None);
    assert_eq!(npc["kind"], "npc");
    assert_eq!(npc["action"], "Pay");
    enter_force_bound_reached(token);
    let failed = call("next", token, p.clone(), Some(json!({ "queued": true })));
    assert_ne!(
        failed["kind"], "loc",
        "walk arrival is not fee proof: {failed}"
    );
    assert!(
        failed["kind"] == "log"
            || failed["message"]
                .as_str()
                .unwrap_or("")
                .contains("took no payment")
            || {
                let again = call("next", token, p.clone(), None);
                again["kind"] == "log"
                    && again["message"]
                        .as_str()
                        .unwrap_or("")
                        .contains("took no payment")
            }
    );

    reset();
    set_observation(seen.clone());
    let token = begin();
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk-near");
    assert_eq!(
        call("next", token, p.clone(), Some(ack_walk(token)))["kind"],
        "status"
    );
    assert_eq!(call("next", token, p.clone(), None)["kind"], "npc");
    seen.inv[0].count = 100;
    seen.locs = vec![EnterLoc {
        id: 5083,
        x: 13,
        z: 12,
        level: 0,
        distance: 1,
    }];
    set_observation(seen);
    let proved = call("next", token, p.clone(), Some(json!({ "queued": true })));
    assert_eq!(proved["kind"], "log");
    assert!(fee_paid_for(token, "brimhaven-iron"));
    let mut saw_loc = false;
    let mut reply = None;
    for _ in 0..12 {
        let step = call("next", token, p.clone(), reply.take());
        if step["kind"] == "loc" {
            saw_loc = true;
            assert_eq!(step["id"], 5083);
            assert_eq!(step["action"], "Enter");
            break;
        }
        if step["kind"] == "yield" {
            break;
        }
    }
    assert!(saw_loc, "fee proof emits loc Enter");
}

#[test]
fn fee_modal_is_close_modal_not_answer() {
    reset();
    let mut seen = obs(Some(tile(12, 12, 0)));
    seen.npcs = vec![EnterNpc {
        index: 3,
        name: "Saniboch".into(),
        actions: vec!["Pay".into()],
        distance: 1,
    }];
    seen.inv = vec![EnterInv {
        id: 995,
        count: 2000,
        slot: 0,
        name: "Coins".into(),
    }];
    seen.main_modal_id = 12;
    set_observation(seen);
    let token = begin();
    let p = fee_site();
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk-near");
    assert_eq!(
        call("next", token, p.clone(), Some(ack_walk(token)))["kind"],
        "status"
    );
    assert_eq!(call("next", token, p.clone(), None)["kind"], "npc");
    let step = call("next", token, p, Some(json!({ "queued": true })));
    assert_eq!(step["kind"], "close-modal");
    assert_ne!(step["kind"], "answer");
    assert_ne!(step["kind"], "answer-count");
}

#[test]
fn prepaid_survives_yield_and_drops_on_reset() {
    reset();
    let mut seen = obs(Some(tile(12, 12, 0)));
    seen.npcs = vec![EnterNpc {
        index: 3,
        name: "Saniboch".into(),
        actions: vec!["Pay".into()],
        distance: 1,
    }];
    seen.inv = vec![EnterInv {
        id: 995,
        count: 1000,
        slot: 0,
        name: "Coins".into(),
    }];
    set_observation(seen.clone());
    let token = begin();
    let p = fee_site();
    let _ = call("next", token, p.clone(), None);
    let _ = call("next", token, p.clone(), None);
    let _ = call("next", token, p.clone(), Some(ack_walk(token)));
    let _ = call("next", token, p.clone(), None);
    seen.chat_lines = vec![EnterChatLine {
        seq: 4,
        text: "You pay Saniboch 875 coins.".into(),
    }];
    set_observation(seen);
    let proved = call("next", token, p.clone(), Some(json!({ "queued": true })));
    assert_eq!(proved["kind"], "log");
    assert!(fee_paid_for(token, "brimhaven-iron"));
    let mut yielded = false;
    for _ in 0..16 {
        let step = call("next", token, p.clone(), None);
        if step["kind"] == "yield" {
            yielded = true;
            assert_eq!(step["value"], false, "entrance missing is not inArea");
            break;
        }
    }
    assert!(yielded);
    assert!(
        fee_paid_for(token, "brimhaven-iron"),
        "yield does not clear prepaid"
    );
    assert!(enter_token_alive(token));
    hunt_fight::on_reset();
    assert!(
        enter_token_alive(token),
        "Fight reset must not drop Enter runtimes"
    );
    hunt_lair::on_reset();
    assert!(!enter_token_alive(token));
    set_observation(obs(Some(outside())));
    let again = begin();
    assert!(!fee_paid_for(again, "brimhaven-iron"));
}

#[test]
fn gateless_completion_is_in_area_not_the_walk_waiter_or_bound() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let approach = json!([{ "x": 5, "z": 5, "level": 0 }]);
    let p = gateless_site(approach);
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    let walk = call("next", token, p.clone(), None);
    assert_eq!(walk["kind"], "walk");
    assert_eq!(walk["x"], 5);
    assert_ne!(walk["kind"], "walk-to");
    assert_ne!(walk["kind"], "loc");
    set_observation(obs(Some(tile(5, 5, 0))));
    assert!(enter_force_bound_reached(token));
    let mut value = None;
    for _ in 0..6 {
        let step = call("next", token, p.clone(), Some(ack_walk(token)));
        if step["kind"] == "yield" {
            value = step["value"].as_bool();
            break;
        }
    }
    assert_eq!(value, Some(false), "approach arrival is not inArea");

    reset();
    set_observation(obs(Some(inside())));
    let token = begin();
    let step = call("next", token, gateless_site(json!([])), None);
    assert_eq!(step["kind"], "yield");
    assert_eq!(step["value"], true);
}

#[test]
fn empty_approach_is_not_a_kbd_substitute() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let p = gateless_site(json!([]));
    let status = call("next", token, p.clone(), None);
    assert_eq!(status["kind"], "status");
    assert_ne!(status["kind"], "aborted");
    let mut walked_ladder = false;
    let mut value = None;
    for _ in 0..6 {
        let step = call("next", token, p.clone(), None);
        if step["x"] == 3017 && step["z"] == 3849 {
            walked_ladder = true;
        }
        if step["kind"] == "yield" {
            value = step["value"].as_bool();
            break;
        }
        assert_ne!(step["kind"], "aborted");
    }
    assert!(!walked_ladder);
    assert_eq!(value, Some(false));
}

#[test]
fn keyed_use_on_emits_the_id_matched_tile_after_radius_0_walk() {
    reset();
    let mut seen = obs(Some(tile(20, 20, 0)));
    seen.inv = vec![EnterInv {
        id: 1590,
        count: 1,
        slot: 3,
        name: "Dusty key".into(),
    }];
    seen.locs = vec![
        EnterLoc {
            id: 999,
            x: 9,
            z: 9,
            level: 0,
            distance: 1,
        },
        EnterLoc {
            id: 2623,
            x: 8,
            z: 8,
            level: 0,
            distance: 2,
        },
    ];
    set_observation(seen);
    let token = begin();
    let p = gate_site(json!({ "name": "Dusty key", "id": 1590 }));
    assert!(validate(token, p.clone()));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    let walk = call("next", token, p.clone(), None);
    assert_eq!(walk["kind"], "walk");
    assert_eq!(walk["x"], 20);
    assert!(walk.get("radius").is_none() || walk["radius"] == 0);
    let delay = call("next", token, p.clone(), Some(ack_walk(token)));
    assert_eq!(delay["kind"], "delay-ticks");
    assert_eq!(delay["n"], 2);
    let used = call("next", token, p, None);
    assert_eq!(used["kind"], "use-on");
    assert_eq!(used["x"], 8);
    assert_eq!(used["z"], 8);
    assert_eq!(used["id"], 1590);
    assert_eq!(used["slot"], 3);
    assert_ne!(used["kind"], "loc");
}

#[test]
fn door_without_key_emits_loc_open_with_id() {
    reset();
    let mut seen = obs(Some(tile(20, 20, 0)));
    seen.locs = vec![EnterLoc {
        id: 2623,
        x: 21,
        z: 20,
        level: 0,
        distance: 1,
    }];
    set_observation(seen);
    let token = begin();
    let p = gate_site(Value::Null);
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk");
    assert_eq!(
        call("next", token, p.clone(), Some(ack_walk(token)))["kind"],
        "delay-ticks"
    );
    let loc = call("next", token, p, None);
    assert_eq!(loc["kind"], "loc");
    assert_eq!(loc["id"], 2623);
    assert_eq!(loc["action"], "Open");
    assert_eq!(loc["x"], 21);
    assert_ne!(loc["kind"], "use-on");
}

#[test]
fn kbd_key_or_route_loc_aborts_without_the_ladder_tile() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let p = with_site(gateless_site(json!([])), json!({ "key": "kbd-lair" }));
    let step = call("next", token, p, None);
    assert_eq!(step["kind"], "aborted");
    assert_eq!(step["reason"], "kbd-later");
    assert_ne!(step["x"], 3017);

    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let p = with_site(
        gateless_site(json!([])),
        json!({ "route": [{ "locId": 1765 }, { "locId": 1816 }] }),
    );
    let step = call("next", token, p, None);
    assert_eq!(step["kind"], "aborted");
    assert_eq!(step["reason"], "kbd-later");
}

#[test]
fn in_area_is_size_1_point_test() {
    reset();
    set_observation(obs(Some(tile(39, 50, 0))));
    let token = begin();
    let p = gateless_site(json!([]));
    let step = call("next", token, p.clone(), None);
    assert_ne!(
        step["kind"], "yield",
        "x=39 is outside a size-1 box starting at 40"
    );
    set_observation(obs(Some(tile(40, 50, 0))));
    let token = begin();
    let step = call("next", token, p, None);
    assert_eq!(step["kind"], "yield");
    assert_eq!(step["value"], true);
}

#[test]
fn pause_freezes_the_one_armed_window() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let p = talk_site();
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk-near");
    let before = enter_deadline_remaining_ms(token).unwrap_or(0);
    assert!(before > 200_000, "stand window is 300000, got {before}");
    hunt_lair::on_pause();
    std::thread::sleep(std::time::Duration::from_millis(280));
    let frozen = call("next", token, p.clone(), Some(ack_walk(token)));
    assert_eq!(frozen["kind"], "wait");
    hunt_lair::on_resume();
    let after = enter_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        (before - after).abs() < 80,
        "stand window elapsed during pause: {before} -> {after}"
    );
}

#[test]
fn hold_yields_false_and_leaves_prepaid_unchanged() {
    reset();
    let mut seen = obs(Some(tile(12, 12, 0)));
    seen.npcs = vec![EnterNpc {
        index: 3,
        name: "Saniboch".into(),
        actions: vec!["Pay".into()],
        distance: 1,
    }];
    seen.inv = vec![EnterInv {
        id: 995,
        count: 1000,
        slot: 0,
        name: "Coins".into(),
    }];
    set_observation(seen.clone());
    let token = begin();
    let p = fee_site();
    let _ = call("next", token, p.clone(), None);
    let _ = call("next", token, p.clone(), None);
    let _ = call("next", token, p.clone(), Some(ack_walk(token)));
    let _ = call("next", token, p.clone(), None);
    seen.inv[0].count = 100;
    set_observation(seen.clone());
    let proved = call("next", token, p.clone(), Some(json!({ "queued": true })));
    assert_eq!(proved["kind"], "log");
    assert!(fee_paid_for(token, "brimhaven-iron"));
    seen.hold = true;
    set_observation(seen);
    let mut value = None;
    for _ in 0..8 {
        let step = call("next", token, p.clone(), None);
        if step["kind"] == "yield" {
            value = step["value"].as_bool();
            break;
        }
    }
    assert_eq!(value, Some(false));
    assert!(fee_paid_for(token, "brimhaven-iron"));
}

#[test]
fn approach_skip_uses_distance_to_so_level_mismatch_does_not_skip() {
    reset();
    set_observation(obs(Some(tile(0, 0, 0))));
    let token = begin();
    let p = gateless_site(json!([
        { "x": 0, "z": 0, "level": 1 },
        { "x": 0, "z": 1, "level": 0 }
    ]));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    let walk = call("next", token, p, None);
    assert_eq!(walk["kind"], "walk");
    assert_eq!(
        walk["level"], 1,
        "level mismatch must not skip the nearest stop"
    );
    assert_eq!(walk["x"], 0);
    assert_eq!(walk["z"], 0);
}

#[test]
fn enter_effects_never_include_forbidden_ops_and_unknown_op_is_not_invented() {
    reset();
    set_observation(obs(Some(outside())));
    let token = begin();
    let p = talk_site();
    let kinds = kinds_until(token, p.clone(), 6);
    for kind in &kinds {
        assert!(!matches!(
            kind.as_str(),
            "walk-to" | "bank-open" | "teleport" | "wait-fed-done" | "attack"
        ));
    }
    let missing = hunt_lair::dispatch(&json!({ "op": "climb-ladder", "token": token }));
    assert_eq!(missing["kind"], "notImpl");
    let eat = call("next", token, p, Some(json!({ "eatOk": true })));
    assert_eq!(eat["kind"], "aborted");
}

#[test]
fn yield_value_and_aborted_shapes_are_not_fight_or_walkspot() {
    reset();
    set_observation(obs(Some(inside())));
    let token = begin();
    let done = call("next", token, gateless_site(json!([])), None);
    assert_eq!(done["kind"], "yield");
    assert!(done["value"].is_boolean());
    assert_eq!(done["value"], true);

    set_observation(obs(Some(outside())));
    let token = begin();
    let aborted = call(
        "next",
        token,
        with_site(gateless_site(json!([])), json!({ "key": "kbd-lair" })),
        None,
    );
    assert_eq!(aborted["kind"], "aborted");
    assert!(aborted.get("value").is_none() || aborted["value"].is_null());
    assert_ne!(aborted["kind"], "yield");
}

#[test]
fn enter_lair_class_walks_with_flags_false_and_leaves_host_fight_alone() {
    let iso = LoadIsolate::spawn(
        r#"
import { EnterLair } from '../../api/combat/hunting/combat.js';
export default class T extends LoopingBot {
    loop() {
        const host = {
            died: false,
            parked: false,
            targetIdx: null,
            hpFraction: () => 1,
            panicHp: () => 0.2,
            retreatHp: () => 0.5,
            hasFood: () => false,
            needEat: () => false,
            style: () => 'range',
            safespotIndex: () => 0,
            buryBones: () => false,
            boneName: () => 'Bones',
            shieldReady: () => true,
            fight: { interruptWatch() { globalThis.__iw = true; } },
            log() {},
            setStatus(m) { globalThis.__status = m; },
        };
        const site = {
            key: 'heroes-blue',
            boxes: [{ minX: 40, maxX: 60, minZ: 40, maxZ: 60, level: 0 }],
            approach: [{ x: 5, z: 5, level: 0 }],
            talkGate: null,
            feeGate: null,
            gate: null,
            keyItem: null,
        };
        const enter = new EnterLair(host, site);
        globalThis.__token = enter.token;
        globalThis.__hostFight = host.fight === enter;
        enter.execute();
    }
}
"#
        .to_string(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&empty_snapshot(
        1,
        TileInput {
            x: 1,
            z: 1,
            level: 0,
        },
    )));
    iso.on_game_tick(1);
    let iw = iso.probe("globalThis.__iw").unwrap_or(Value::Null);
    let host_fight = iso.probe("globalThis.__hostFight").unwrap();
    let drained = iso.drain_interacts();
    iso.join();
    assert_ne!(iw, true, "EnterLair must not interruptWatch");
    assert_eq!(host_fight, false, "EnterLair must not assign host.fight");
    assert!(
        drained.iter().any(|req| matches!(
            req,
            InteractReq::Walk {
                x: 5,
                z: 5,
                level: 0,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id,
            } if *request_id != 0
        )),
        "{drained:?}"
    );
}

// ResetSession drops the Rust policy state while the isolate, the card state
// and the JS tokens survive: a hunt class that gated on validate before the
// reset must not stay inert. Rust starts the token's state fresh on its next
// use (no JS re-bind), so validate reads true again and execute walks.
#[test]
fn session_reset_keeps_the_class_token_live_on_fresh_state() {
    let src = r#"
import { EnterLair, HoldSafespot } from '../../api/combat/hunting/combat.js';
export default class T extends LoopingBot {
    loop() {
        const host = {
            died: false,
            parked: false,
            targetIdx: null,
            hpFraction: () => 1,
            panicHp: () => 0.2,
            retreatHp: () => 0.5,
            hasFood: () => false,
            needEat: () => false,
            style: () => 'range',
            safespotIndex: () => 0,
            buryBones: () => false,
            boneName: () => 'Bones',
            shieldReady: () => true,
            log() {},
            setStatus(m) { globalThis.__status = m; },
            setSafespotIndex() {},
        };
        const site = {
            key: 'heroes-blue',
            target: 'Goblin',
            alsoHunt: [],
            safespots: [{ x: 2901, z: 9809, level: 0 }],
            meleeAnchor: { x: 2900, z: 9808, level: 0 },
            boxes: [{ minX: 40, maxX: 60, minZ: 40, maxZ: 60, level: 0 }],
            fireAtRange: false,
            rangedThreat: false,
            approach: [{ x: 5, z: 5, level: 0 }],
            talkGate: null,
            feeGate: null,
            gate: null,
            keyItem: null,
        };
        const enter = globalThis.__enter || (globalThis.__enter = new EnterLair(host, site));
        const hold = globalThis.__hold || (globalThis.__hold = new HoldSafespot(host, site));
        globalThis.__ticks = (globalThis.__ticks || 0) + 1;
        globalThis.__firstEnter = globalThis.__firstEnter || enter.token;
        globalThis.__firstHold = globalThis.__firstHold || hold.token;
        globalThis.__validate = enter.validate();
        hold.validate();
        globalThis.__enterToken = enter.token;
        globalThis.__holdToken = hold.token;
        // Only the post-reset tick walks: the walk step parks the isolate
        // until a real host settles it, and the loop must run again after the
        // reset for the re-bind to be observable.
        if (globalThis.__validate && globalThis.__ticks > 1) enter.execute();
    }
}
"#;
    let here = || {
        empty_snapshot(
            0,
            TileInput {
                x: 1,
                z: 1,
                level: 0,
            },
        )
    };
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = here();
    snap.tick = 1;
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&snap));
    iso.on_game_tick(1);
    let first_enter = iso.probe("__firstEnter").unwrap();
    let first_hold = iso.probe("__firstHold").unwrap();
    let validated = iso.probe("__validate").unwrap();
    let ticks = iso.probe("__ticks").unwrap();
    let before = iso.drain_interacts();
    assert_eq!(validated, true, "the pre-reset token validates");
    assert_eq!(ticks, 1, "the first tick only validates: {ticks:?}");
    assert!(
        before.is_empty(),
        "the pre-reset tick must not start the walk: {before:?}"
    );
    assert_ne!(first_enter, Value::Null, "{first_enter:?}");
    assert_ne!(first_hold, Value::Null, "{first_hold:?}");

    iso.reset_session_work();
    let mut snap = here();
    snap.tick = 2;
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&snap));
    iso.on_game_tick(2);
    let enter_token = iso.probe("__enterToken").unwrap();
    let hold_token = iso.probe("__holdToken").unwrap();
    let validated = iso.probe("__validate").unwrap();
    let ticks = iso.probe("__ticks").unwrap();
    let after = iso.drain_interacts();
    let logs = iso.join();
    assert_eq!(ticks, 2, "the loop runs again after the reset: {logs:?}");
    assert_eq!(
        validated, true,
        "post-reset validate reads the fresh state: {logs:?}"
    );
    assert_eq!(enter_token, first_enter, "the class keeps its token");
    assert_eq!(hold_token, first_hold, "the class keeps its token");
    assert!(
        approach_walk(&after),
        "execute runs on the new token: {after:?} logs={logs:?}"
    );
}

fn approach_walk(drained: &[InteractReq]) -> bool {
    drained.iter().any(|req| {
        matches!(
            req,
            InteractReq::Walk {
                x: 5,
                z: 5,
                level: 0,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id,
            } if *request_id != 0
        )
    })
}

#[test]
fn v2_enter_run_settles_the_yield_value_and_refuses_an_unknown_token() {
    let src = r#"
export const apiVersion = 2;
const site = {
  key: 'heroes-blue',
  boxes: [{ minX: 0, maxX: 10, minZ: 0, maxZ: 10, level: 0 }],
  approach: [],
};
let token = null;
export async function tick(api) {
  if (token !== null) return;
  const began = api.enterBegin(site);
  globalThis.__begin = began;
  token = began.value.token;
  globalThis.__validate = api.enterValidate({ token }, { shieldReady: () => true });
  globalThis.__bad = await api.enterRun({ token: token + 1 }, {});
  globalThis.__done = await api.enterRun({ token }, {});
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    for tick in 1..=3 {
        iso.post_snapshot(script::isolate_fb::encode_snapshot(&empty_snapshot(
            tick,
            TileInput {
                x: 1,
                z: 1,
                level: 0,
            },
        )));
        iso.on_game_tick(tick);
    }
    let begin = iso.probe("globalThis.__begin").unwrap();
    assert_eq!(begin["ok"], true, "{begin:?}");
    // Already inside the area: validate is false (the old projection read).
    let validate = iso.probe("globalThis.__validate").unwrap();
    assert_eq!(validate, json!({ "ok": true, "value": false }));
    let bad = iso.probe("globalThis.__bad").unwrap();
    assert_eq!(bad, json!({ "kind": "refused", "reason": "unknown token" }));
    // Inside the boxes: the run settles done with the yield's value.
    let done = iso.probe("globalThis.__done").unwrap();
    assert_eq!(done, json!({ "kind": "done", "value": true }));
    iso.join();
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
