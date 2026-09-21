//! Offline Retreat isolate policy tests. No LIVE.

use script::hunt_fight::{
    self, hold_token_alive, last_hp, retreat_deadline_remaining_ms, retreat_force_bound_reached,
    retreat_token_alive, seen_contains, skip_contains, token_alive, FightNpc, FightObservation,
    Tile, RETREAT_HOP_MS, RETREAT_RETRY_MS,
};
use script::isolate_fb::{SnapshotInput, TileInput};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use serde_json::{json, Value};
use std::thread;
use std::time::Duration;

fn reset() {
    hunt_fight::on_reset();
}

fn box_site() -> Value {
    json!({
        "key": "test",
        "target": "Goblin",
        "alsoHunt": [],
        "safespots": [
            {"x": 2901, "z": 9809, "level": 0},
            {"x": 2902, "z": 9810, "level": 0}
        ],
        "meleeAnchor": {"x": 2900, "z": 9808, "level": 0},
        "boxes": [{"minX": 2888, "maxX": 2923, "minZ": 9769, "maxZ": 9816, "level": 0}],
        "fireAtRange": false,
        "rangedThreat": false,
    })
}

fn proj(extra: Value) -> Value {
    let mut v = json!({
        "died": false,
        "targetIdx": null,
        "hpFraction": 1.0,
        "panicHp": 0.2,
        "retreatHp": 0.5,
        "hasFood": true,
        "needEat": false,
        "style": "melee",
        "safespotIndex": 0,
        "buryBones": false,
        "boneName": "Bones",
        "hasVlog": false,
        "hasArmSpecial": true,
        "hasShieldReady": false,
        "shieldReady": false,
    });
    if let Some(obj) = v.as_object_mut() {
        if let Some(site) = box_site().as_object() {
            for (k, val) in site {
                obj.insert(k.clone(), val.clone());
            }
        }
        if let Some(extra) = extra.as_object() {
            for (k, val) in extra {
                obj.insert(k.clone(), val.clone());
            }
        }
    }
    v
}

fn goblin(distance: i32) -> FightNpc {
    FightNpc {
        index: 7,
        id: 1,
        name: "Goblin".into(),
        x: 2901,
        z: 9808,
        level: 0,
        nx: 2901,
        nz: 9808,
        size: 1,
        distance,
        health: 50,
        in_combat: false,
        actions: vec!["Attack".into()],
        target_kind: 0,
        target_index: -1,
    }
}

fn obs_at(here: Tile) -> FightObservation {
    FightObservation {
        here: Some(here),
        ingame: true,
        scene_state: 2,
        hold: false,
        ours: false,
        chat_continue: false,
        npcs: vec![],
        self_target_kind: 0,
        self_target_index: -1,
        self_slot: 0,
        hp_effective: 70,
        inv_names: Vec::new(),
        animating: false,
        los_override: Some(true),
        tick: 1,
    }
}

fn melee_anchor() -> Tile {
    Tile {
        x: 2900,
        z: 9808,
        level: 0,
    }
}

fn off_spot() -> Tile {
    Tile {
        x: 2895,
        z: 9808,
        level: 0,
    }
}

fn closer_to_spot_1() -> Tile {
    Tile {
        x: 2904,
        z: 9810,
        level: 0,
    }
}

fn safespot_0() -> Tile {
    Tile {
        x: 2901,
        z: 9809,
        level: 0,
    }
}

fn begin() -> u64 {
    hunt_fight::retreat_dispatch(&json!({ "op": "begin" }))["token"]
        .as_u64()
        .expect("token")
}

fn fight_begin() -> u64 {
    hunt_fight::dispatch(&json!({ "op": "begin" }))["token"]
        .as_u64()
        .expect("token")
}

fn hold_begin() -> u64 {
    hunt_fight::hold_dispatch(&json!({ "op": "begin" }))["token"]
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
    hunt_fight::retreat_dispatch(&v)
}

fn fight_call(op: &str, token: u64, p: Value, reply: Option<Value>) -> Value {
    let mut v = p;
    v["op"] = json!(op);
    v["token"] = json!(token);
    if let Some(r) = reply {
        v["reply"] = r;
    }
    hunt_fight::dispatch(&v)
}

fn hold_call(op: &str, token: u64, p: Value, reply: Option<Value>) -> Value {
    let mut v = p;
    v["op"] = json!(op);
    v["token"] = json!(token);
    if let Some(r) = reply {
        v["reply"] = r;
    }
    hunt_fight::hold_dispatch(&v)
}

fn validate(token: u64, p: Value) -> bool {
    call("validate", token, p, None)["value"]
        .as_bool()
        .unwrap_or(false)
}

fn foodless_off() -> Value {
    proj(json!({
        "hasFood": false,
        "needEat": false,
        "hpFraction": 0.4,
        "retreatHp": 0.5,
    }))
}

fn prelude(token: u64, p: &Value) -> Value {
    let set = call("next", token, p.clone(), None);
    assert_eq!(set["kind"], "set-safespot", "{set}");
    let status = call("next", token, p.clone(), None);
    assert_eq!(status["kind"], "status", "{status}");
    let log = call("next", token, p.clone(), None);
    assert_eq!(log["kind"], "log", "{log}");
    set
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

#[test]
fn foodless_off_safespot_validates() {
    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let token = begin();
    assert!(validate(token, foodless_off()));
}

#[test]
fn foodless_on_safespot_is_false() {
    reset();
    hunt_fight::set_observation(obs_at(safespot_0()));
    let token = begin();
    assert!(!validate(token, foodless_off()));
}

#[test]
fn fed_below_retreat_hp_off_spot_ignores_need_eat() {
    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let token = begin();
    assert!(validate(
        token,
        proj(json!({
            "hasFood": true,
            "needEat": true,
            "hpFraction": 0.4,
            "retreatHp": 0.5,
        }))
    ));
    assert!(validate(
        token,
        proj(json!({
            "hasFood": true,
            "needEat": false,
            "hpFraction": 0.4,
            "retreatHp": 0.5,
        }))
    ));
}

#[test]
fn fed_at_or_above_retreat_hp_is_false() {
    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let token = begin();
    assert!(!validate(
        token,
        proj(json!({
            "hasFood": true,
            "needEat": false,
            "hpFraction": 0.5,
            "retreatHp": 0.5,
        }))
    ));
}

#[test]
fn retreat_hp_out_of_area_no_spots_and_here_null_are_false() {
    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let token = begin();
    assert!(!validate(
        token,
        proj(json!({ "hasFood": false, "retreatHp": 0.0 }))
    ));
    hunt_fight::set_observation(obs_at(Tile {
        x: 0,
        z: 0,
        level: 0,
    }));
    assert!(!validate(token, foodless_off()));
    hunt_fight::set_observation(obs_at(off_spot()));
    let mut p = foodless_off();
    p["safespots"] = json!([]);
    assert!(!validate(token, p));
    hunt_fight::set_observation(FightObservation {
        here: None,
        ..obs_at(off_spot())
    });
    assert!(!validate(token, foodless_off()));
}

#[test]
fn retry_gate_blocks_until_freeze_aware_elapsed() {
    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let token = begin();
    let p = foodless_off();
    prelude(token, &p);
    for _ in 0..4 {
        let hop = call("next", token, p.clone(), None);
        assert_eq!(hop["kind"], "walk-to", "{hop}");
        assert!(retreat_force_bound_reached(token));
    }
    let log = call("next", token, p.clone(), None);
    assert_eq!(log["kind"], "log", "{log}");
    let set = call("next", token, p.clone(), None);
    assert_eq!(set["kind"], "set-safespot", "{set}");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "yield");
    assert!(!validate(token, p.clone()));
    assert!(retreat_force_bound_reached(token));
    assert!(validate(token, p));
}

#[test]
fn execute_interrupt_watch_keeps_skip_seen_and_distinct_tokens() {
    reset();
    hunt_fight::set_observation(FightObservation {
        npcs: vec![goblin(1)],
        ..obs_at(melee_anchor())
    });
    let fight = fight_begin();
    let p = proj(json!({}));
    let mut reply = None;
    for _ in 0..16 {
        let step = fight_call("next", fight, p.clone(), reply.take());
        if step["kind"] == "npc" {
            reply = Some(json!({ "queued": false }));
            continue;
        }
        if step["kind"] == "yield" || step["kind"] == "aborted" {
            break;
        }
        reply = match step["kind"].as_str().unwrap_or("") {
            "eat" => Some(json!({ "eatOk": true })),
            "npc" => Some(json!({ "queued": true })),
            "bury" => Some(json!({ "buried": true })),
            _ => None,
        };
        if skip_contains(fight, 7) && seen_contains(fight, 7) {
            break;
        }
    }
    assert!(skip_contains(fight, 7));
    assert!(seen_contains(fight, 7));
    hunt_fight::set_observation(obs_at(off_spot()));
    let hold = hold_begin();
    let retreat = begin();
    assert_ne!(retreat, fight);
    assert_ne!(retreat, hold);
    assert_ne!(hold, fight);
    fight_call("interruptWatch", fight, p, None);
    assert_eq!(last_hp(fight), Some(-1));
    assert!(skip_contains(fight, 7));
    assert!(seen_contains(fight, 7));
    assert!(token_alive(fight));
    assert!(hold_token_alive(hold));
    assert!(retreat_token_alive(retreat));
    let bad = call("next", retreat + 1, foodless_off(), None);
    assert_eq!(bad["kind"], "aborted");
    hunt_fight::on_reset();
    assert!(!retreat_token_alive(retreat));
}

#[test]
fn aim_fresh_uses_nearest_safespot_never_melee_anchor() {
    reset();
    hunt_fight::set_observation(obs_at(closer_to_spot_1()));
    let token = begin();
    let p = foodless_off();
    let set = prelude(token, &p);
    assert_eq!(set["index"], 1);
    let hop = call("next", token, p, None);
    assert_eq!(hop["kind"], "walk-to");
    assert_eq!(hop["x"], 2902);
    assert_eq!(hop["z"], 9810);
    assert_eq!(hop["level"], 0);
    assert_ne!(hop["x"], 2900);
    assert_ne!(hop["z"], 9808);
}

#[test]
fn aim_rotated_keeps_failed_ladder_not_nearest() {
    reset();
    hunt_fight::set_observation(obs_at(closer_to_spot_1()));
    let token = begin();
    let p = foodless_off();
    let first = prelude(token, &p);
    assert_eq!(first["index"], 1);
    for _ in 0..4 {
        assert_eq!(call("next", token, p.clone(), None)["kind"], "walk-to");
        assert!(retreat_force_bound_reached(token));
    }
    assert_eq!(call("next", token, p.clone(), None)["kind"], "log");
    let fail_set = call("next", token, p.clone(), None);
    assert_eq!(fail_set["kind"], "set-safespot");
    assert_eq!(fail_set["index"], 0);
    assert_eq!(call("next", token, p.clone(), None)["kind"], "yield");
    assert!(retreat_force_bound_reached(token));
    let set = prelude(token, &p);
    assert_eq!(set["index"], 0, "rotated must stick even if nearest is 1");
    let hop = call("next", token, p, None);
    assert_eq!(hop["kind"], "walk-to");
    assert_eq!(hop["x"], 2901);
    assert_eq!(hop["z"], 9809);
}

#[test]
fn hop_kind_is_walk_to_never_walk_or_npc() {
    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let token = begin();
    let p = foodless_off();
    let mut kinds = Vec::new();
    for _ in 0..8 {
        let step = call("next", token, p.clone(), None);
        let kind = step["kind"].as_str().unwrap_or("").to_string();
        kinds.push(kind.clone());
        assert_ne!(kind, "walk");
        assert_ne!(kind, "npc");
        assert_ne!(kind, "walk-near");
        assert_ne!(kind, "wait-fed-done");
        if kind == "walk-to" {
            assert_eq!(step["x"], 2901);
            assert_eq!(step["z"], 9809);
            break;
        }
        if kind == "yield" || kind == "aborted" {
            break;
        }
    }
    assert!(kinds.iter().any(|k| k == "walk-to"), "{kinds:?}");
}

#[test]
fn first_hop_emits_walk_to_even_if_hold_or_ours() {
    reset();
    let mut held = obs_at(off_spot());
    held.hold = true;
    hunt_fight::set_observation(held);
    let token = begin();
    let p = foodless_off();
    prelude(token, &p);
    let hop = call("next", token, p.clone(), None);
    assert_eq!(hop["kind"], "walk-to", "first hop must go out before yield");
    assert_eq!(hop["x"], 2901);
    assert_eq!(hop["z"], 9809);
    let step = call("next", token, p, None);
    assert_eq!(step["kind"], "yield");
}

#[test]
fn arrival_after_hop_yields_without_rotate() {
    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let token = begin();
    let p = foodless_off();
    prelude(token, &p);
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk-to");
    hunt_fight::set_observation(obs_at(safespot_0()));
    let step = call("next", token, p.clone(), None);
    assert_eq!(step["kind"], "yield");
    assert!(!validate(
        token,
        proj(json!({ "hasFood": false, "hpFraction": 0.4 }))
    ));
}

#[test]
fn four_missed_hops_rotate_and_arm_retry() {
    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let token = begin();
    let p = foodless_off();
    let set = prelude(token, &p);
    assert_eq!(set["index"], 0);
    for _ in 0..4 {
        let hop = call("next", token, p.clone(), None);
        assert_eq!(hop["kind"], "walk-to", "{hop}");
        assert_eq!(hop["x"], 2901);
        assert_ne!(hop["kind"], "walk");
        assert!(retreat_force_bound_reached(token));
    }
    assert_eq!(call("next", token, p.clone(), None)["kind"], "log");
    let rotate = call("next", token, p.clone(), None);
    assert_eq!(rotate["kind"], "set-safespot");
    assert_eq!(rotate["index"], 1);
    assert_eq!(call("next", token, p.clone(), None)["kind"], "yield");
    let remain = retreat_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        remain > (RETREAT_RETRY_MS as i64) - 200,
        "retry armed: {remain}"
    );
    assert!(!validate(token, p));
}

#[test]
fn died_mid_hop_yields_without_rotate() {
    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let token = begin();
    let p = foodless_off();
    prelude(token, &p);
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk-to");
    let died = proj(json!({
        "died": true,
        "hasFood": false,
        "hpFraction": 0.4,
        "retreatHp": 0.5,
    }));
    let step = call("next", token, died, None);
    assert_eq!(step["kind"], "yield");
    assert!(validate(token, foodless_off()), "died must not arm retry");
    let set = call("next", token, foodless_off(), None);
    assert_eq!(set["kind"], "set-safespot");
}

#[test]
fn hop_wait_emits_sustain_then_delay_ticks() {
    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let token = begin();
    let p = foodless_off();
    prelude(token, &p);
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk-to");
    let sustain = call("next", token, p.clone(), None);
    assert_eq!(sustain["kind"], "sustain");
    let delay = call("next", token, p, None);
    assert_eq!(delay["kind"], "delay-ticks");
    assert_eq!(delay["n"], 1);
    assert_ne!(delay["kind"], "wait-fed-done");
}

#[test]
fn pause_hold_does_not_elapse_hop_or_retry() {
    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let token = begin();
    let p = foodless_off();
    prelude(token, &p);
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk-to");
    let before = retreat_deadline_remaining_ms(token).unwrap_or(0);
    assert!(before > (RETREAT_HOP_MS as i64) - 200, "{before}");
    hunt_fight::on_pause();
    thread::sleep(Duration::from_millis(280));
    hunt_fight::on_resume();
    let after = retreat_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        (before - after).abs() < 80,
        "HOP_MS elapsed during freeze: {before} -> {after}"
    );
}

#[test]
fn leftover_walk_token_does_not_settle_retreat() {
    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let token = begin();
    let p = foodless_off();
    prelude(token, &p);
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk-to");
    let abort = call(
        "next",
        token,
        p.clone(),
        Some(json!({ "walkToken": 99, "queued": true })),
    );
    assert_eq!(abort["kind"], "aborted", "{abort}");

    let token = begin();
    prelude(token, &p);
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk-to");
    let sustain = call("next", token, p, None);
    assert_eq!(sustain["kind"], "sustain");
    assert_ne!(sustain["kind"], "yield");
}

#[test]
fn class_switches_stay_split_and_walk_to_spot_is_live() {
    let src = include_str!("../src/shim/hunting_combat.js");
    assert!(!src.contains("retreatDue"));
    assert!(!src.contains("retreatAim"));
    assert!(!src.contains("nearestSpot"));
    let fight = src
        .split("export class Retreat")
        .next()
        .expect("fight class");
    assert!(fight.contains("case 'walk-to':"));
    assert!(!fight.contains("case 'walk':"));
    let retreat = src
        .split("export class Retreat")
        .nth(1)
        .unwrap()
        .split("export class HoldSafespot")
        .next()
        .expect("retreat class");
    assert!(retreat.contains("case 'walk-to':"));
    assert!(!retreat.contains("case 'walk':"));
    assert!(retreat.contains("this.host.fight?.interruptWatch()"));
    let hold = src
        .split("export class HoldSafespot")
        .nth(1)
        .unwrap()
        .split("export class WalkToSpot")
        .next()
        .expect("hold class");
    assert!(hold.contains("case 'walk':"));
    assert!(!hold.contains("case 'walk-to':"));

    let iso = LoadIsolate::spawn(
        r#"
import { Retreat, WalkToSpot } from '../../api/combat/hunting/combat.js';
export default class T extends LoopingBot {
    loop() {
        const probe = { walkToSpot: null };
        try { new WalkToSpot(); } catch (e) { probe.walkToSpot = String(e && e.message || e); }
        globalThis.__probe = JSON.stringify(probe);
    }
}
"#
        .to_string(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    iso.on_game_tick(1);
    let probe: Value =
        serde_json::from_str(iso.probe("__probe").unwrap().as_str().unwrap()).unwrap();
    iso.join();
    assert!(
        probe["walkToSpot"].is_null()
            || !probe["walkToSpot"]
                .as_str()
                .unwrap_or("")
                .contains("not impl"),
        "WalkToSpot isolate is live: {probe:?}"
    );
}

#[test]
fn retreat_next_never_emits_walk_and_siblings_keep_verbs() {
    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let retreat = begin();
    let p = foodless_off();
    let mut kinds = Vec::new();
    for _ in 0..10 {
        let step = call("next", retreat, p.clone(), None);
        let kind = step["kind"].as_str().unwrap_or("").to_string();
        kinds.push(kind.clone());
        assert_ne!(kind, "walk");
        if kind == "walk-to" || kind == "yield" || kind == "aborted" {
            break;
        }
    }
    assert!(kinds.iter().any(|k| k == "walk-to"), "{kinds:?}");

    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let hold = hold_begin();
    let hp = proj(json!({}));
    let mut hold_kinds = Vec::new();
    let mut reply = None;
    for _ in 0..8 {
        let step = hold_call("next", hold, hp.clone(), reply.take());
        let kind = step["kind"].as_str().unwrap_or("").to_string();
        hold_kinds.push(kind.clone());
        assert_ne!(kind, "walk-to");
        if kind == "walk" {
            reply = Some(json!({ "queued": true, "walkToken": 17 }));
        } else if kind == "yield" || kind == "aborted" {
            break;
        }
    }
    assert!(hold_kinds.iter().any(|k| k == "walk"), "{hold_kinds:?}");

    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let fight = fight_begin();
    let fp = proj(json!({
        "hasFood": false,
        "needEat": false,
        "hpFraction": 0.4,
        "retreatHp": 0.5,
    }));
    let mut fight_kinds = Vec::new();
    for _ in 0..8 {
        let step = fight_call("next", fight, fp.clone(), None);
        let kind = step["kind"].as_str().unwrap_or("").to_string();
        fight_kinds.push(kind.clone());
        assert_ne!(kind, "walk");
        if kind == "yield" || kind == "aborted" {
            break;
        }
    }
    assert_eq!(
        fight_kinds.last().map(String::as_str),
        Some("yield"),
        "empty-pack retreatDue gate unchanged: {fight_kinds:?}"
    );

    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let fight = fight_begin();
    let fed = proj(json!({
        "hasFood": true,
        "needEat": false,
        "hpFraction": 0.4,
        "retreatHp": 0.5,
    }));
    let mut fed_kinds = Vec::new();
    for _ in 0..8 {
        let step = fight_call("next", fight, fed.clone(), None);
        let kind = step["kind"].as_str().unwrap_or("").to_string();
        fed_kinds.push(kind.clone());
        if kind == "yield" || kind == "npc" || kind == "arm-special" || kind == "aborted" {
            break;
        }
    }
    assert_ne!(
        fed_kinds.get(1).map(String::as_str),
        Some("yield"),
        "fed !needEat below retreatHp must not reopen Fight yield: {fed_kinds:?}"
    );
}

#[test]
fn unexpected_eatok_aborts() {
    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let token = begin();
    let p = foodless_off();
    prelude(token, &p);
    let bad = call("next", token, p, Some(json!({ "eatOk": true })));
    assert_eq!(bad["kind"], "aborted");
}

#[test]
fn empty_spots_and_here_null_yield_without_panic() {
    reset();
    hunt_fight::set_observation(obs_at(off_spot()));
    let token = begin();
    let mut p = foodless_off();
    p["safespots"] = json!([]);
    let step = call("next", token, p, None);
    assert_eq!(step["kind"], "yield");

    hunt_fight::set_observation(FightObservation {
        here: None,
        ..obs_at(off_spot())
    });
    let token = begin();
    let step = call("next", token, foodless_off(), None);
    assert_eq!(step["kind"], "yield");
}

#[test]
fn v1_execute_calls_interrupt_watch_and_queues_walk_to() {
    let iso = LoadIsolate::spawn(
        r#"
import { Retreat } from '../../api/combat/hunting/combat.js';
export default class T extends LoopingBot {
    loop() {
        const host = {
            died: false,
            targetIdx: null,
            hpFraction: () => 0.4,
            panicHp: () => 0.2,
            retreatHp: () => 0.5,
            hasFood: () => false,
            needEat: () => false,
            style: () => 'melee',
            safespotIndex: () => 0,
            buryBones: () => false,
            boneName: () => 'Bones',
            fight: { interruptWatch() { globalThis.__iw = true; } },
            log() {},
            setStatus(m) { globalThis.__status = m; },
            setSafespotIndex(n) { globalThis.__spot = n; },
        };
        const site = {
            key: 'test',
            target: 'Goblin',
            alsoHunt: [],
            safespots: [
                { x: 2901, z: 9809, level: 0 },
                { x: 2902, z: 9810, level: 0 },
            ],
            meleeAnchor: { x: 2900, z: 9808, level: 0 },
            boxes: [{ minX: 2888, maxX: 2923, minZ: 9769, maxZ: 9816, level: 0 }],
            fireAtRange: false,
            rangedThreat: false,
        };
        const retreat = new Retreat(host, site);
        globalThis.__token = retreat.token;
        globalThis.__hostFight = host.fight === retreat;
        retreat.execute();
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
            x: 2895,
            z: 9808,
            level: 0,
        },
    )));
    iso.on_game_tick(1);
    let iw = iso.probe("globalThis.__iw").unwrap();
    let host_fight = iso.probe("globalThis.__hostFight").unwrap();
    let drained = iso.drain_interacts();
    iso.join();
    assert_eq!(iw, true, "execute must call host.fight.interruptWatch");
    assert_eq!(host_fight, false, "Retreat must not assign host.fight");
    let walks: Vec<_> = drained
        .into_iter()
        .filter(|req| matches!(req, InteractReq::WalkTo { .. } | InteractReq::Walk { .. }))
        .collect();
    assert!(
        walks.iter().any(|req| matches!(
            req,
            InteractReq::WalkTo {
                x: 2901,
                z: 9809,
                level: 0,
            }
        )),
        "{walks:?}"
    );
    assert!(
        !walks
            .iter()
            .any(|req| matches!(req, InteractReq::Walk { .. })),
        "{walks:?}"
    );
}
