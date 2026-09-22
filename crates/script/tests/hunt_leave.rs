//! Offline Leave isolate policy tests. No LIVE.
//!
//! These lock the architecture corrections: KBD abort, yield shape, rune-id
//! precheck, gate true-bit, and walk-flag falses. They do not mirror phases.

use api::game_data::{self, SelectedGameData, TeleportSpell};
use client::io::ClientRevision;
use script::hunt_fight::Tile;
use script::hunt_leave::{
    self, leave_deadline_remaining_ms, leave_force_bound_reached, leave_token_alive,
    set_observation, LeaveInv, LeaveLoc, LeaveObservation, DOOR_MS, WALK_LEG_MS,
};
use script::isolate_fb::{
    encode_snapshot, ItemRowInput, SnapshotInput, SnapshotReader, StatInput, TileInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use serde_json::{json, Value};
use std::sync::Arc;

fn reset() {
    hunt_leave::on_reset();
}

fn tile(x: i32, z: i32, level: i32) -> Tile {
    Tile { x, z, level }
}

fn inside() -> Tile {
    tile(50, 50, 0)
}

fn outside() -> Tile {
    tile(1, 1, 0)
}

fn boxes() -> Value {
    json!([{ "minX": 40, "maxX": 60, "minZ": 40, "maxZ": 60, "level": 0 }])
}

fn cache() -> Arc<SelectedGameData> {
    game_data::for_revision(ClientRevision::R274).expect("selected cache")
}

fn falador(data: &SelectedGameData) -> &TeleportSpell {
    data.teleport("falador").expect("falador row")
}

fn obs(here: Option<Tile>, magic_base: Option<i32>, inv: Vec<LeaveInv>) -> LeaveObservation {
    LeaveObservation {
        here,
        ingame: true,
        hold: false,
        ours: false,
        locs: vec![],
        inv,
        magic_base,
        tick: 1,
    }
}

fn rows_by_id(spell: &TeleportSpell) -> Vec<LeaveInv> {
    spell
        .runes
        .iter()
        .map(|rune| LeaveInv {
            id: rune.id,
            count: rune.count,
            name: format!("not-{}", rune.name),
        })
        .collect()
}

fn rows_by_name(spell: &TeleportSpell) -> Vec<LeaveInv> {
    spell
        .runes
        .iter()
        .enumerate()
        .map(|(i, rune)| LeaveInv {
            id: 90_000 + i as i32,
            count: rune.count.max(1) + 5,
            name: rune.name.clone(),
        })
        .collect()
}

fn with_staff(mut rows: Vec<LeaveInv>, spell: &TeleportSpell) -> Vec<LeaveInv> {
    if let Some(rune) = spell.runes.first() {
        rows.push(LeaveInv {
            id: 1,
            count: 1,
            name: format!("Staff of {}", rune.name),
        });
    }
    rows
}

fn site(extra: Value) -> Value {
    let mut base = json!({
        "key": "heroes-blue",
        "boxes": boxes(),
        "leaveByWalk": false,
        "escapeTeleportId": "falador",
        "walkOut": { "x": 1, "z": 1, "level": 0 },
        "exit": null,
        "gate": null,
    });
    if let Some(obj) = extra.as_object() {
        for (k, v) in obj {
            base[k] = v.clone();
        }
    }
    base
}

fn begin() -> u64 {
    hunt_leave::dispatch(None, &json!({ "op": "begin" }))["token"]
        .as_u64()
        .expect("token")
}

fn call(data: Option<&SelectedGameData>, token: u64, mut p: Value, reply: Option<Value>) -> Value {
    p["op"] = json!("next");
    p["token"] = json!(token);
    if let Some(r) = reply {
        p["reply"] = r;
    }
    hunt_leave::dispatch(data, &p)
}

fn walk_reply(token: u64) -> Value {
    json!({ "queued": true, "walkToken": token.wrapping_add(1000) })
}

fn kind(step: &Value) -> &str {
    step["kind"].as_str().unwrap_or("")
}

#[test]
fn already_out_yields_true_with_no_ops() {
    reset();
    set_observation(obs(Some(outside()), Some(99), vec![]));
    let token = begin();
    let step = call(None, token, site(json!({ "leaveByWalk": true })), None);
    assert_eq!(kind(&step), "yield");
    assert_eq!(step["value"], true);
    assert!(step.get("x").is_none());
}

#[test]
fn already_out_kbd_key_yields_true_without_abort() {
    reset();
    set_observation(obs(Some(outside()), Some(99), vec![]));
    let token = begin();
    let step = call(
        None,
        token,
        site(json!({
            "key": "kbd-lair",
            "escapeTeleportId": "varrock",
            "leaveByWalk": true,
        })),
        None,
    );
    assert_eq!(kind(&step), "yield", "{step}");
    assert_eq!(step["value"], true);
    assert_ne!(step["reason"], "kbd-later");
    assert!(step.get("x").is_none());
}

#[test]
fn already_out_kbd_loc_yields_true_without_abort() {
    reset();
    set_observation(obs(Some(outside()), Some(99), vec![]));
    let token = begin();
    let step = call(
        None,
        token,
        site(json!({
            "leaveByWalk": true,
            "exit": {
                "locId": 1765,
                "op": "Pull",
                "stand": { "x": 50, "z": 50, "level": 0 }
            },
        })),
        None,
    );
    assert_eq!(kind(&step), "yield", "{step}");
    assert_eq!(step["value"], true);
    assert_ne!(step["reason"], "kbd-later");
    assert!(step.get("x").is_none());
}

#[test]
fn already_out_missing_leave_by_walk_yields_true_without_abort() {
    reset();
    set_observation(obs(Some(outside()), None, vec![]));
    let token = begin();
    let mut p = site(json!({}));
    p.as_object_mut().unwrap().remove("leaveByWalk");
    let step = call(None, token, p, None);
    assert_eq!(kind(&step), "yield", "{step}");
    assert_eq!(step["value"], true);
    assert_ne!(step["reason"], "missing leaveByWalk");
    assert_ne!(kind(&step), "teleport");
    assert!(step.get("x").is_none());
}

#[test]
fn missing_leave_by_walk_aborts_instead_of_casting() {
    reset();
    let data = cache();
    let spell = falador(&data);
    set_observation(obs(Some(inside()), Some(spell.level), rows_by_id(spell)));
    let token = begin();
    let mut p = site(json!({}));
    p.as_object_mut().unwrap().remove("leaveByWalk");
    let step = call(Some(&data), token, p, None);
    assert_eq!(kind(&step), "aborted");
    assert_eq!(step["reason"], "missing leaveByWalk");
    assert_ne!(kind(&step), "teleport");
}

#[test]
fn leave_by_walk_skips_teleport_even_when_runes_suffice() {
    reset();
    let data = cache();
    let spell = falador(&data);
    set_observation(obs(Some(inside()), Some(spell.level), rows_by_id(spell)));
    let token = begin();
    let step = call(
        Some(&data),
        token,
        site(json!({ "leaveByWalk": true })),
        None,
    );
    assert_eq!(kind(&step), "walk-near");
    assert_eq!(step["radius"], 3);
    assert_ne!(kind(&step), "teleport");
}

#[test]
fn teleport_names_the_escape_id_and_does_not_invent_a_button() {
    reset();
    let data = cache();
    let spell = falador(&data);
    set_observation(obs(Some(inside()), Some(spell.level), rows_by_id(spell)));
    let token = begin();
    let step = call(Some(&data), token, site(json!({})), None);
    assert_eq!(kind(&step), "teleport");
    assert_eq!(step["name"], "falador");
    assert!(step.get("component_id").is_none());
    assert_ne!(step["name"], spell.name);
}

#[test]
fn rune_precheck_matches_ids_not_names_and_ignores_a_staff() {
    reset();
    let data = cache();
    let spell = falador(&data).clone();
    set_observation(obs(
        Some(inside()),
        Some(spell.level),
        with_staff(rows_by_name(&spell), &spell),
    ));
    let token = begin();
    let named = call(Some(&data), token, site(json!({})), None);
    assert_ne!(
        kind(&named),
        "teleport",
        "display names must not satisfy SpellRune.id"
    );
    assert!(matches!(kind(&named), "log" | "walk" | "walk-near"));

    reset();
    set_observation(obs(
        Some(inside()),
        Some(spell.level),
        with_staff(rows_by_id(&spell), &spell),
    ));
    let token = begin();
    let by_id = call(Some(&data), token, site(json!({})), None);
    assert_eq!(kind(&by_id), "teleport");
    assert_eq!(by_id["name"], "falador");
}

#[test]
fn magic_gate_reads_base_not_effective() {
    reset();
    let data = cache();
    let spell = falador(&data).clone();
    let inv: Vec<ItemRowInput> = spell
        .runes
        .iter()
        .enumerate()
        .map(|(i, rune)| ItemRowInput {
            name: Some("wrong-name"),
            count: rune.count,
            id: rune.id,
            ops: &[],
            noted: false,
            cert: -1,
            component_id: -1,
            slot: i as i32,
        })
        .collect();
    let low = [StatInput {
        index: 6,
        name: "Magic",
        xp: 0,
        base: 1,
        effective: 99,
    }];
    let bytes = encode_snapshot(&snap_with(inside(), &low, &inv));
    let reader = SnapshotReader::from_bytes(&bytes).unwrap();
    hunt_leave::on_snapshot(&reader);
    let token = begin();
    let short = call(Some(&data), token, site(json!({})), None);
    assert_ne!(
        kind(&short),
        "teleport",
        "effective 99 must not pass a base-1 gate"
    );

    reset();
    let high = [StatInput {
        index: 6,
        name: "Magic",
        xp: 0,
        base: spell.level,
        effective: 1,
    }];
    let bytes = encode_snapshot(&snap_with(inside(), &high, &inv));
    let reader = SnapshotReader::from_bytes(&bytes).unwrap();
    hunt_leave::on_snapshot(&reader);
    let token = begin();
    let ready = call(Some(&data), token, site(json!({})), None);
    assert_eq!(kind(&ready), "teleport");
}

#[test]
fn unknown_id_falls_through_and_is_not_blocked() {
    reset();
    set_observation(obs(Some(inside()), Some(99), vec![]));
    let token = begin();
    let step = call(
        Some(&cache()),
        token,
        site(json!({ "escapeTeleportId": "not-a-spell" })),
        None,
    );
    assert_ne!(kind(&step), "aborted");
    assert_ne!(step["reason"], "BLOCKED: missing teleport");
    let next = if kind(&step) == "log" {
        call(
            Some(&cache()),
            token,
            site(json!({ "escapeTeleportId": "not-a-spell" })),
            None,
        )
    } else {
        step
    };
    assert_eq!(kind(&next), "walk-near");
    assert_ne!(kind(&next), "teleport");
}

#[test]
fn helper_done_while_inside_is_not_left() {
    reset();
    let data = cache();
    let spell = falador(&data).clone();
    set_observation(obs(Some(inside()), Some(spell.level), rows_by_id(&spell)));
    let token = begin();
    let p = site(json!({}));
    assert_eq!(kind(&call(Some(&data), token, p.clone(), None)), "teleport");
    let step = call(Some(&data), token, p, Some(json!({ "teleported": true })));
    assert_ne!(kind(&step), "yield");
    assert_ne!(step["value"], true);
    assert!(matches!(kind(&step), "sustain" | "delay-ticks" | "wait"));
}

#[test]
fn left_despite_helper_false_yields_true_without_a_walk() {
    reset();
    let data = cache();
    let spell = falador(&data).clone();
    set_observation(obs(Some(inside()), Some(spell.level), rows_by_id(&spell)));
    let token = begin();
    let p = site(json!({}));
    assert_eq!(kind(&call(Some(&data), token, p.clone(), None)), "teleport");
    set_observation(obs(Some(outside()), Some(spell.level), rows_by_id(&spell)));
    let step = call(Some(&data), token, p, Some(json!({ "teleported": false })));
    assert_eq!(kind(&step), "yield");
    assert_eq!(step["value"], true);
}

#[test]
fn three_failed_casts_then_walk_out_and_no_fourth() {
    reset();
    let data = cache();
    let spell = falador(&data).clone();
    set_observation(obs(Some(inside()), Some(spell.level), rows_by_id(&spell)));
    let token = begin();
    let p = site(json!({}));
    let mut teleports = 0;
    let mut reply = None;
    let mut walked = false;
    for _ in 0..24 {
        let step = call(Some(&data), token, p.clone(), reply.take());
        match kind(&step) {
            "teleport" => {
                teleports += 1;
                reply = Some(json!({ "teleported": false }));
            }
            "sustain" => {
                assert!(leave_force_bound_reached(token));
            }
            "walk" | "walk-near" => {
                walked = true;
                break;
            }
            "yield" | "aborted" => panic!("ended before walk-out: {step}"),
            _ => {}
        }
    }
    assert_eq!(teleports, 3);
    assert!(walked, "third failure must fall through to walk-out");
}

#[test]
fn mid_loop_shortfall_stops_further_teleports() {
    reset();
    let data = cache();
    let spell = falador(&data).clone();
    set_observation(obs(Some(inside()), Some(spell.level), rows_by_id(&spell)));
    let token = begin();
    let p = site(json!({}));
    assert_eq!(kind(&call(Some(&data), token, p.clone(), None)), "teleport");
    let polled = call(
        Some(&data),
        token,
        p.clone(),
        Some(json!({ "teleported": false })),
    );
    assert_eq!(kind(&polled), "sustain");
    set_observation(obs(Some(inside()), None, rows_by_id(&spell)));
    assert!(leave_force_bound_reached(token));
    let step = call(Some(&data), token, p.clone(), None);
    assert_ne!(kind(&step), "teleport");
    assert_ne!(step["n"], 3);
    let follow = if kind(&step) == "log" {
        call(Some(&data), token, p, None)
    } else {
        step
    };
    assert!(matches!(kind(&follow), "walk" | "walk-near"));
}

#[test]
fn exit_loc_uses_the_posted_op_and_never_walks_walk_out() {
    reset();
    set_observation(obs(Some(inside()), None, vec![]));
    let token = begin();
    let p = site(json!({
        "leaveByWalk": true,
        "exit": {
            "locId": 5084,
            "op": "leave",
            "stand": { "x": 50, "z": 50, "level": 0 }
        },
        "gate": {
            "locId": 2623,
            "op": "Open",
            "inside": { "x": 48, "z": 50, "level": 0 }
        },
        "walkOut": { "x": 9, "z": 9, "level": 0 }
    }));
    let walk = call(None, token, p.clone(), None);
    assert_eq!(kind(&walk), "walk-near");
    assert_eq!(walk["radius"], 2);
    assert_eq!(walk["x"], 50);
    assert_ne!(walk["x"], 9);
    let delayed = call(None, token, p.clone(), Some(walk_reply(token)));
    assert_eq!(kind(&delayed), "delay-ticks");
    assert_eq!(delayed["n"], 1);
    let mut seen = obs(Some(inside()), None, vec![]);
    seen.locs = vec![LeaveLoc {
        id: 5084,
        x: 50,
        z: 51,
        level: 0,
        distance: 1,
    }];
    set_observation(seen);
    let loc = call(None, token, p.clone(), None);
    assert_eq!(kind(&loc), "loc");
    assert_eq!(loc["action"], "leave");
    assert_ne!(loc["action"], "Leave");
    assert_ne!(loc["action"], "Open");
    let polled = call(None, token, p.clone(), Some(json!({ "queued": true })));
    assert_eq!(kind(&polled), "sustain");
    assert!(leave_force_bound_reached(token));
    let done = call(None, token, p, None);
    assert_eq!(kind(&done), "yield");
    assert_eq!(done["value"], false);
}

#[test]
fn missing_exit_loc_yields_false_and_does_not_substitute_the_gate() {
    reset();
    set_observation(obs(Some(inside()), None, vec![]));
    let token = begin();
    let p = site(json!({
        "leaveByWalk": true,
        "exit": {
            "locId": 2813,
            "op": "Enter",
            "stand": { "x": 50, "z": 50, "level": 0 }
        },
        "gate": {
            "locId": 2623,
            "op": "Open",
            "inside": { "x": 48, "z": 50, "level": 0 }
        }
    }));
    let _walk = call(None, token, p.clone(), None);
    let delayed = call(None, token, p.clone(), Some(walk_reply(token)));
    assert_eq!(kind(&delayed), "delay-ticks");
    let missing = call(None, token, p, None);
    assert_eq!(kind(&missing), "yield");
    assert_eq!(missing["value"], false);
}

#[test]
fn gate_open_true_bit_is_out_of_area_not_the_walk_out() {
    reset();
    set_observation(obs(Some(inside()), None, vec![]));
    let token = begin();
    let p = gate_site(1, 1);
    let walk = call(None, token, p.clone(), None);
    assert_eq!(kind(&walk), "walk");
    assert!(walk.get("radius").is_none());
    assert_eq!(walk["x"], 50);
    let delayed = call(None, token, p.clone(), Some(walk_reply(token)));
    assert_eq!(delayed["n"], 2);
    let mut seen = obs(Some(inside()), None, vec![]);
    seen.locs = vec![LeaveLoc {
        id: 2623,
        x: 50,
        z: 51,
        level: 0,
        distance: 1,
    }];
    set_observation(seen);
    let loc = call(None, token, p.clone(), None);
    assert_eq!(kind(&loc), "loc");
    assert_eq!(loc["action"], "Open");
    assert_eq!(loc["id"], 2623);
    let still = call(None, token, p.clone(), Some(json!({ "queued": true })));
    assert_eq!(kind(&still), "sustain");
    assert!(leave_force_bound_reached(token));
    let failed = call(None, token, p, None);
    assert_eq!(kind(&failed), "yield");
    assert_eq!(failed["value"], false);
    assert_ne!(failed["x"], 1);
}

#[test]
fn failed_walk_out_while_already_out_yields_true() {
    reset();
    set_observation(obs(Some(inside()), None, vec![]));
    let token = begin();
    let p = gate_site(9, 9);
    open_gate(token, &p);
    set_observation(obs(Some(outside()), None, vec![]));
    let trailed = call(None, token, p.clone(), Some(json!({ "queued": true })));
    assert_eq!(kind(&trailed), "walk-near");
    assert_eq!(trailed["radius"], 3);
    assert_eq!(trailed["x"], 9);
    assert!(leave_force_bound_reached(token));
    let done = call(None, token, p, Some(walk_reply(token)));
    assert_eq!(kind(&done), "yield");
    assert_eq!(done["value"], true);
}

#[test]
fn walk_out_arrival_while_still_inside_yields_false() {
    reset();
    set_observation(obs(Some(inside()), None, vec![]));
    let token = begin();
    let p = gate_site(50, 52);
    open_gate(token, &p);
    set_observation(obs(Some(outside()), None, vec![]));
    let trailed = call(None, token, p.clone(), Some(json!({ "queued": true })));
    assert_eq!(kind(&trailed), "walk-near");
    set_observation(obs(Some(tile(50, 52, 0)), None, vec![]));
    let done = call(None, token, p, Some(walk_reply(token)));
    assert_eq!(kind(&done), "yield");
    assert_eq!(done["value"], false);
}

#[test]
fn occupied_gate_stand_uses_walk_near_radius_2() {
    reset();
    set_observation(obs(Some(tile(48, 50, 0)), None, vec![]));
    let token = begin();
    let p = gate_site(1, 1);
    let walk = call(None, token, p.clone(), None);
    assert_eq!(kind(&walk), "walk");
    assert!(leave_force_bound_reached(token));
    let logged = call(None, token, p.clone(), Some(walk_reply(token)));
    assert_eq!(kind(&logged), "log");
    let near = call(None, token, p, None);
    assert_eq!(kind(&near), "walk-near");
    assert_eq!(near["radius"], 2);
    assert_ne!(kind(&near), "walk-to");
}

#[test]
fn gateless_is_walk_near_radius_3_not_a_sixth_kind() {
    reset();
    set_observation(obs(Some(inside()), None, vec![]));
    let token = begin();
    let step = call(None, token, site(json!({ "leaveByWalk": true })), None);
    assert_eq!(kind(&step), "walk-near");
    assert_eq!(step["radius"], 3);
    assert_eq!(step["x"], 1);
    assert_ne!(kind(&step), "loc");
}

#[test]
fn kbd_key_route_or_loc_aborts_before_teleport_or_walk() {
    reset();
    let data = cache();
    let spell = falador(&data).clone();
    set_observation(obs(Some(inside()), Some(99), rows_by_id(&spell)));
    let edgeville = json!({ "x": 3094, "z": 3493, "level": 0 });
    for extra in [
        json!({ "key": "kbd-lair", "escapeTeleportId": "varrock", "walkOut": edgeville }),
        json!({ "route": { "outLever": { "locId": 1 } }, "escapeTeleportId": "varrock" }),
        json!({ "leaveByWalk": true, "exit": { "locId": 1817, "op": "Pull", "stand": { "x": 50, "z": 50, "level": 0 } } }),
        json!({ "outLever": { "id": 1766 } }),
    ] {
        reset();
        set_observation(obs(Some(inside()), Some(99), rows_by_id(&spell)));
        let token = begin();
        let step = call(Some(&data), token, site(extra), None);
        assert_eq!(kind(&step), "aborted", "{step}");
        assert_eq!(step["reason"], "kbd-later");
        assert_ne!(kind(&step), "teleport");
        assert_ne!(kind(&step), "walk");
        assert_ne!(kind(&step), "walk-near");
    }
}

#[test]
fn size_one_point_outside_the_box_is_already_out() {
    reset();
    set_observation(obs(Some(tile(39, 50, 0)), None, vec![]));
    let token = begin();
    let step = call(None, token, site(json!({ "leaveByWalk": true })), None);
    assert_eq!(kind(&step), "yield");
    assert_eq!(step["value"], true, "x=39 is outside a size-1 box");

    reset();
    set_observation(obs(Some(tile(40, 50, 0)), None, vec![]));
    let token = begin();
    let step = call(None, token, site(json!({ "leaveByWalk": true })), None);
    assert_ne!(kind(&step), "yield");
    assert_eq!(kind(&step), "walk-near");
}

#[test]
fn enter_pause_does_not_freeze_the_leave_door_window() {
    reset();
    let data = cache();
    let spell = falador(&data).clone();
    set_observation(obs(Some(inside()), Some(spell.level), rows_by_id(&spell)));
    let token = begin();
    let p = site(json!({}));
    assert_eq!(kind(&call(Some(&data), token, p.clone(), None)), "teleport");
    let polled = call(
        Some(&data),
        token,
        p.clone(),
        Some(json!({ "teleported": false })),
    );
    assert_eq!(kind(&polled), "sustain");
    let before = leave_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        before > 7_000 && before <= DOOR_MS as i64,
        "proof window must be DOOR_MS {DOOR_MS}, got {before}"
    );
    assert!(before < 120_000, "must not arm FIGHT_MS");
    script::hunt_lair::on_pause();
    script::hunt_fight::on_pause();
    std::thread::sleep(std::time::Duration::from_millis(220));
    let stepped = call(Some(&data), token, p.clone(), None);
    assert_ne!(
        kind(&stepped),
        "wait",
        "Enter/Fight pause must not freeze Leave"
    );
    let mid = leave_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        before - mid > 80,
        "Leave clock elapsed under Enter pause: {before} -> {mid}"
    );
    hunt_leave::on_pause();
    std::thread::sleep(std::time::Duration::from_millis(280));
    let frozen = call(Some(&data), token, p, None);
    assert_eq!(kind(&frozen), "wait");
    hunt_leave::on_resume();
    let after = leave_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        (mid - after).abs() < 80,
        "DOOR_MS elapsed during Leave pause: {mid} -> {after}"
    );
}

#[test]
fn walk_leg_is_not_the_fight_window() {
    reset();
    set_observation(obs(Some(inside()), None, vec![]));
    let token = begin();
    let step = call(None, token, site(json!({ "leaveByWalk": true })), None);
    assert_eq!(kind(&step), "walk-near");
    let left = leave_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        left > 200_000 && left <= WALK_LEG_MS as i64,
        "walk leg must be {WALK_LEG_MS}, got {left}"
    );
}

#[test]
fn hold_during_proof_yields_false() {
    reset();
    let data = cache();
    let spell = falador(&data).clone();
    set_observation(obs(Some(inside()), Some(spell.level), rows_by_id(&spell)));
    let token = begin();
    let p = site(json!({}));
    assert_eq!(kind(&call(Some(&data), token, p.clone(), None)), "teleport");
    let mut held = obs(Some(outside()), Some(spell.level), rows_by_id(&spell));
    held.hold = true;
    set_observation(held);
    let step = call(Some(&data), token, p, Some(json!({ "teleported": true })));
    assert_eq!(kind(&step), "yield");
    assert_eq!(step["value"], false);
}

#[test]
fn leave_effects_never_include_forbidden_ops() {
    reset();
    set_observation(obs(Some(inside()), None, vec![]));
    let token = begin();
    let p = site(json!({ "leaveByWalk": true }));
    let step = call(None, token, p.clone(), None);
    assert!(!matches!(
        kind(&step),
        "walk-to" | "use-on" | "npc" | "bank-open" | "wait-fed-done" | "attack"
    ));
    let eat = call(None, token, p, Some(json!({ "eatOk": true })));
    assert_eq!(kind(&eat), "aborted");
    let src = include_str!("../src/hunt_leave.rs");
    assert!(!src.contains("teleport::dispatch"));
    assert!(!src.contains("escape_runes"));
    assert!(!src.contains("provided_runes"));
    assert!(!src.contains("gap_sw"));
    assert!(!src.contains("bodyOrigin"));
    assert!(!src.contains("size>>1"));
    assert!(!src.contains("size >> 1"));
    assert!(!src.contains("FIGHT_MS"));
    assert!(!src.contains("APPROACH_MS"));
    assert!(!src.contains("APPROACH_LEG_MS"));
    assert!(!src.contains("hunt_lair"));
    assert!(!src.contains("effective("));
    assert!(!src.contains("3094"));
    assert!(!src.contains("varrock"));
    assert!(src.contains("in_area_body(here, 1,"));
}

#[test]
fn yield_shape_and_shim_flags_are_not_fight_copies() {
    let bindings = include_str!("../src/load/bindings.rs");
    let leave = bindings.split("api.leaveNext").nth(1).expect("leaveNext");
    let leave = leave.split("function recordSettlement").next().unwrap();
    assert!(leave.contains("kind: 'yield', value:"));
    assert!(leave.contains("ok: false, error:"));
    assert!(leave.contains("kind: 'aborted'"));
    assert!(leave.contains("status: 'aborted'"));
    assert!(!leave.contains("ok: true, status: 'aborted'"));
    assert!(!bindings.contains("api.leaveValidate"));

    let dts = include_str!("../src/host_js.rs");
    let step = dts
        .split("export type LeaveStep")
        .nth(1)
        .expect("LeaveStep");
    assert!(step.contains("kind: 'yield'; value: boolean"));
    assert!(step.contains("ok: false; error: string; kind: 'aborted'"));
    assert!(!step.contains("leaveValidate"));

    let js = include_str!("../src/shim/hunting_combat.js");
    let enter = js.split("export class EnterLair").nth(1).unwrap();
    let enter = enter.split("function leaveCall").next().unwrap();
    assert!(!enter.contains("case 'teleport'"));
    assert!(!enter.contains("leaveByWalk"));
    assert!(!enter.contains("escapeTeleportId"));
    let shared = js.split("function projection(").nth(1).unwrap();
    let shared = shared.split("export class Fight").next().unwrap();
    assert!(!shared.contains("leaveByWalk"));
    assert!(!shared.contains("walkOut"));
    let leave_js = js.split("function leaveCall").nth(1).expect("leaveCall");
    assert!(leave_js.contains("allow_teleports: false"));
    assert!(leave_js.contains("allow_wilderness: false"));
    assert!(leave_js.contains("allow_bank_fetch: false"));
    assert!(leave_js.contains("__rs2b0t_teleport"));
    assert!(leave_js.contains("leaveProjection"));
    assert!(leave_js.contains("runLeaveTeleport"));
    assert!(!leave_js.contains("Game.teleport"));
    assert!(!leave_js.contains("inArea"));
    assert!(!leave_js.contains("case 'walk-to'"));
    assert!(!leave_js.contains("interruptWatch"));
    let begin = js.split("function beginLeaveWalk").nth(1).unwrap();
    let begin = begin
        .split("async function runLeaveTeleport")
        .next()
        .unwrap();
    assert!(begin.contains("allow_wilderness: false"));
    assert!(begin.contains("allow_bank_fetch: false"));
    assert!(begin.contains("allow_teleports: false"));

    let iso = include_str!("../src/load/isolate.rs");
    for hook in [
        "on_snapshot",
        "on_pause",
        "on_hold",
        "on_resume",
        "on_reset",
    ] {
        assert!(
            iso.contains(&format!("hunt_leave::{hook}")),
            "missing {hook}"
        );
        assert!(iso.contains(&format!("hunt_lair::{hook}")));
    }
}

#[test]
fn wrong_token_aborts_and_reset_drops_the_map() {
    reset();
    let token = begin();
    assert!(leave_token_alive(token));
    let bad = call(None, token + 9, site(json!({ "leaveByWalk": true })), None);
    assert_eq!(kind(&bad), "aborted");
    assert_eq!(bad["reason"], "unknown token");
    hunt_leave::on_reset();
    assert!(!leave_token_alive(token));
}

#[test]
fn queued_leave_walk_flags_are_false() {
    let src = r#"
import { leaveLair } from '../../api/combat/hunting/combat.js';
export default class T extends LoopingBot {
    loop() {
        const host = {
            leaveByWalk() { return true; },
            fight: { id: 7 },
            log() {},
            setStatus() {},
        };
        const site = {
            key: 'heroes-blue',
            boxes: [{ minX: 40, maxX: 60, minZ: 40, maxZ: 60, level: 0 }],
            escapeTeleportId: 'falador',
            walkOut: { x: 1, z: 1, level: 0 },
            exit: null,
            gate: null,
        };
        leaveLair(host, site);
        globalThis.__fight = host.fight.id;
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    iso.post_snapshot(encode_snapshot(&empty_snapshot(
        1,
        TileInput {
            x: 50,
            z: 50,
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
                x: 1,
                z: 1,
                level: 0,
                radius: 3,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id,
            } if *request_id != 0
        )),
        "{drained:?}"
    );
    assert!(drained.iter().all(|req| match req {
        InteractReq::Walk {
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            ..
        }
        | InteractReq::WalkNear {
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            ..
        } => !allow_teleports && !allow_wilderness && !allow_bank_fetch,
        _ => true,
    }));
}

#[test]
fn v2_leave_next_yield_carries_value_and_aborted_is_not_done() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const began = api.leaveBegin();
  globalThis.__begin = began;
  const token = began.value.token;
  const bad = api.leaveNext({
    token: token + 1,
    leaveByWalk: true,
    key: 'heroes-blue',
    boxes: [{ minX: 40, maxX: 60, minZ: 40, maxZ: 60, level: 0 }],
  });
  globalThis.__bad = bad;
  const done = api.leaveNext({
    token,
    leaveByWalk: true,
    key: 'heroes-blue',
    boxes: [{ minX: 40, maxX: 60, minZ: 40, maxZ: 60, level: 0 }],
    walkOut: { x: 1, z: 1, level: 0 },
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
    assert_eq!(bad["kind"], "aborted", "{bad:?}");
    assert_eq!(bad["status"], "aborted", "{bad:?}");
    assert_ne!(bad["status"], "done");
    let done = iso.probe("globalThis.__done").unwrap();
    assert_eq!(done["ok"], true, "{done:?}");
    assert_eq!(done["status"], "done", "{done:?}");
    assert_eq!(done["kind"], "yield", "{done:?}");
    assert_eq!(done["value"], true, "{done:?}");
    iso.join();
}

fn gate_site(x: i32, z: i32) -> Value {
    site(json!({
        "leaveByWalk": true,
        "key": "taverley-blue",
        "gate": {
            "locId": 2623,
            "op": "Open",
            "inside": { "x": 50, "z": 50, "level": 0 }
        },
        "walkOut": { "x": x, "z": z, "level": 0 }
    }))
}

fn open_gate(token: u64, p: &Value) {
    let walk = call(None, token, p.clone(), None);
    assert_eq!(kind(&walk), "walk", "{walk}");
    let delayed = call(None, token, p.clone(), Some(walk_reply(token)));
    assert_eq!(kind(&delayed), "delay-ticks");
    let mut seen = obs(Some(inside()), None, vec![]);
    seen.locs = vec![LeaveLoc {
        id: 2623,
        x: 50,
        z: 51,
        level: 0,
        distance: 1,
    }];
    set_observation(seen);
    let loc = call(None, token, p.clone(), None);
    assert_eq!(kind(&loc), "loc", "{loc}");
}

fn snap_with<'a>(
    here: Tile,
    stats: &'a [StatInput<'a>],
    inv: &'a [ItemRowInput<'a>],
) -> SnapshotInput<'a> {
    let mut snap = empty_snapshot(
        1,
        TileInput {
            x: here.x,
            z: here.z,
            level: here.level,
        },
    );
    snap.stats = stats;
    snap.inv = inv;
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
