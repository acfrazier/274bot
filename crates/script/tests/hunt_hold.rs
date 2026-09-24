//! Offline HoldSafespot isolate policy tests. No LIVE.

use script::hunt_fight::{
    self, hold_deadline_remaining_ms, hold_force_bound_reached, hold_token_alive, last_hp,
    seen_contains, skip_contains, token_alive, FightNpc, FightObservation, Tile,
};
use script::isolate_fb::{ItemRowInput, SnapshotInput, StatInput, TileInput};
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

fn off_anchor() -> Tile {
    Tile {
        x: 2895,
        z: 9808,
        level: 0,
    }
}

fn safespot_a() -> Tile {
    Tile {
        x: 2901,
        z: 9809,
        level: 0,
    }
}

fn begin() -> u64 {
    hunt_fight::hold_dispatch(&json!({ "op": "begin" }))["token"]
        .as_u64()
        .expect("token")
}

fn fight_begin() -> u64 {
    hunt_fight::dispatch(&json!({ "op": "begin" }))["token"]
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
    hunt_fight::hold_dispatch(&v)
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

fn validate(token: u64, p: Value) -> bool {
    call("validate", token, p, None)["value"]
        .as_bool()
        .unwrap_or(false)
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
fn chase_gate_blocks_when_melee_fire_at_range_has_target() {
    reset();
    hunt_fight::set_observation(obs_at(off_anchor()));
    let token = begin();
    let p = proj(json!({
        "style": "melee",
        "fireAtRange": true,
        "targetIdx": 7,
        "hasFood": true,
    }));
    assert!(!validate(token, p));
}

#[test]
fn chase_without_target_validates() {
    reset();
    hunt_fight::set_observation(obs_at(off_anchor()));
    let token = begin();
    let p = proj(json!({
        "style": "melee",
        "fireAtRange": true,
        "targetIdx": null,
        "hasFood": true,
    }));
    assert!(validate(token, p));
}

#[test]
fn foodless_on_safespot_is_not_due() {
    reset();
    hunt_fight::set_observation(obs_at(safespot_a()));
    let token = begin();
    let p = proj(json!({
        "style": "melee",
        "hasFood": false,
    }));
    assert!(!validate(token, p));
}

#[test]
fn foodless_off_every_safespot_is_due() {
    reset();
    hunt_fight::set_observation(obs_at(off_anchor()));
    let token = begin();
    let p = proj(json!({
        "style": "melee",
        "hasFood": false,
    }));
    assert!(validate(token, p));
}

#[test]
fn range_on_other_safespot_is_due() {
    reset();
    hunt_fight::set_observation(obs_at(safespot_a()));
    let token = begin();
    let p = proj(json!({
        "style": "range",
        "safespotIndex": 1,
        "hasFood": true,
    }));
    assert!(validate(token, p));
}

#[test]
fn already_on_anchor_is_false() {
    reset();
    hunt_fight::set_observation(obs_at(melee_anchor()));
    let token = begin();
    assert!(!validate(token, proj(json!({}))));
}

#[test]
fn out_of_area_and_panic_hp_are_false() {
    reset();
    hunt_fight::set_observation(obs_at(Tile {
        x: 0,
        z: 0,
        level: 0,
    }));
    let token = begin();
    assert!(!validate(token, proj(json!({}))));
    hunt_fight::set_observation(obs_at(off_anchor()));
    assert!(!validate(
        token,
        proj(json!({ "hpFraction": 0.1, "panicHp": 0.2 }))
    ));
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
    hunt_fight::set_observation(obs_at(off_anchor()));
    let hold = begin();
    assert_ne!(hold, fight);
    fight_call("interruptWatch", fight, p, None);
    assert_eq!(last_hp(fight), Some(-1));
    assert!(skip_contains(fight, 7));
    assert!(seen_contains(fight, 7));
    assert!(token_alive(fight));
    assert!(hold_token_alive(hold));
}

#[test]
fn first_actions_are_status_then_walk_never_walk_to_or_npc() {
    reset();
    hunt_fight::set_observation(obs_at(off_anchor()));
    let token = begin();
    let p = proj(json!({}));
    let status = call("next", token, p.clone(), None);
    let walk = call("next", token, p, None);
    assert_eq!(status["kind"], "status");
    assert_eq!(walk["kind"], "walk");
    assert_eq!(walk["x"], 2900);
    assert_eq!(walk["z"], 9808);
    assert_eq!(walk["level"], 0);
    assert_ne!(walk["kind"], "walk-to");
    assert_ne!(walk["kind"], "npc");
}

#[test]
fn arrival_after_walk_ack_yields_without_rotate() {
    reset();
    hunt_fight::set_observation(obs_at(off_anchor()));
    let token = begin();
    let p = proj(json!({}));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk");
    hunt_fight::set_observation(obs_at(melee_anchor()));
    let step = call(
        "next",
        token,
        p,
        Some(json!({ "queued": true, "walkToken": 11 })),
    );
    assert_eq!(step["kind"], "yield");
    assert_ne!(step["kind"], "set-safespot");
}

#[test]
fn timeout_rotates_range_and_rewrites_melee_index() {
    reset();
    hunt_fight::set_observation(obs_at(off_anchor()));
    let token = begin();
    let p = proj(json!({ "style": "range", "safespotIndex": 0 }));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    let walk = call("next", token, p.clone(), None);
    assert_eq!(walk["kind"], "walk");
    assert_eq!(walk["x"], 2901);
    assert_eq!(walk["z"], 9809);
    assert!(hold_force_bound_reached(token));
    let log = call(
        "next",
        token,
        p.clone(),
        Some(json!({ "queued": true, "walkToken": 12 })),
    );
    assert_eq!(log["kind"], "log", "{log}");
    let set = call("next", token, p.clone(), None);
    assert_eq!(set["kind"], "set-safespot");
    assert_eq!(set["index"], 1);
    assert_eq!(call("next", token, p, None)["kind"], "yield");

    reset();
    hunt_fight::set_observation(obs_at(off_anchor()));
    let melee = begin();
    let mp = proj(json!({ "style": "melee" }));
    assert_eq!(call("next", melee, mp.clone(), None)["kind"], "status");
    assert_eq!(call("next", melee, mp.clone(), None)["kind"], "walk");
    assert!(hold_force_bound_reached(melee));
    let log = call(
        "next",
        melee,
        mp.clone(),
        Some(json!({ "queued": true, "walkToken": 13 })),
    );
    assert_eq!(log["kind"], "log");
    let set = call("next", melee, mp.clone(), None);
    assert_eq!(set["kind"], "set-safespot");
    assert_eq!(set["index"], 0);
    assert_eq!(call("next", melee, mp, None)["kind"], "yield");
}

#[test]
fn leftover_here_does_not_yield_before_walk() {
    reset();
    hunt_fight::set_observation(obs_at(melee_anchor()));
    let token = begin();
    let p = proj(json!({}));
    let status = call("next", token, p.clone(), None);
    let walk = call("next", token, p.clone(), None);
    assert_eq!(status["kind"], "status");
    assert_eq!(walk["kind"], "walk", "leftover here must not skip walk");
    let arrived = call(
        "next",
        token,
        p,
        Some(json!({ "queued": true, "walkToken": 14 })),
    );
    assert_eq!(arrived["kind"], "yield");
}

#[test]
fn waiting_emits_sustain_then_delay_ticks() {
    reset();
    hunt_fight::set_observation(obs_at(off_anchor()));
    let token = begin();
    let p = proj(json!({}));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk");
    let sustain = call(
        "next",
        token,
        p.clone(),
        Some(json!({ "queued": true, "walkToken": 15 })),
    );
    assert_eq!(sustain["kind"], "sustain");
    let delay = call("next", token, p, None);
    assert_eq!(delay["kind"], "delay-ticks");
    assert_eq!(delay["n"], 1);
}

#[test]
fn pause_hold_does_not_elapse_return_ms() {
    reset();
    hunt_fight::set_observation(obs_at(off_anchor()));
    let token = begin();
    let p = proj(json!({}));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    let before = hold_deadline_remaining_ms(token).unwrap_or(0);
    assert!(before > 50_000, "{before}");
    hunt_fight::on_pause();
    thread::sleep(Duration::from_millis(280));
    hunt_fight::on_resume();
    let after = hold_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        (before - after).abs() < 80,
        "RETURN_MS elapsed during freeze: {before} -> {after}"
    );
}

#[test]
fn event_signal_mid_wait_yields_without_rotate() {
    reset();
    hunt_fight::set_observation(obs_at(off_anchor()));
    let token = begin();
    let p = proj(json!({ "style": "range" }));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk");
    let mut held = obs_at(off_anchor());
    held.hold = true;
    hunt_fight::set_observation(held);
    let step = call(
        "next",
        token,
        p,
        Some(json!({ "queued": true, "walkToken": 16 })),
    );
    assert_eq!(step["kind"], "yield");
}

#[test]
fn hold_next_never_emits_walk_to_and_fight_next_never_emits_walk() {
    reset();
    hunt_fight::set_observation(obs_at(off_anchor()));
    let hold = begin();
    let p = proj(json!({}));
    let mut reply = None;
    let mut hold_kinds = Vec::new();
    for _ in 0..8 {
        let step = call("next", hold, p.clone(), reply.take());
        let kind = step["kind"].as_str().unwrap_or("").to_string();
        hold_kinds.push(kind.clone());
        if kind == "walk" {
            reply = Some(json!({ "queued": true, "walkToken": 17 }));
        } else if kind == "yield" || kind == "aborted" {
            break;
        }
        assert_ne!(kind, "walk-to");
        assert_ne!(kind, "npc");
        assert_ne!(kind, "eat");
    }
    assert!(hold_kinds.iter().any(|k| k == "walk"), "{hold_kinds:?}");

    reset();
    hunt_fight::set_observation(obs_at(off_anchor()));
    let fight = fight_begin();
    let mut fight_kinds = Vec::new();
    let mut reply = None;
    for _ in 0..12 {
        let step = fight_call("next", fight, p.clone(), reply.take());
        let kind = step["kind"].as_str().unwrap_or("").to_string();
        fight_kinds.push(kind.clone());
        if kind == "walk-to" || kind == "yield" || kind == "aborted" {
            break;
        }
        reply = None;
        assert_ne!(kind, "walk");
    }
    assert!(
        fight_kinds.iter().any(|k| k == "walk-to"),
        "{fight_kinds:?}"
    );
    assert!(!fight_kinds.iter().any(|k| k == "walk"), "{fight_kinds:?}");
}

#[test]
fn unexpected_eatok_and_npc_queued_abort() {
    reset();
    hunt_fight::set_observation(obs_at(off_anchor()));
    let token = begin();
    let p = proj(json!({}));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    let bad = call("next", token, p.clone(), Some(json!({ "eatOk": true })));
    assert_eq!(bad["kind"], "aborted");

    let token = begin();
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk");
    let npc = call("next", token, p, Some(json!({ "queued": true })));
    assert_eq!(npc["kind"], "aborted");
}

#[test]
fn wrong_token_aborts_and_session_reset_drops_hold() {
    reset();
    hunt_fight::set_observation(obs_at(off_anchor()));
    let token = begin();
    let p = proj(json!({}));
    let bad = call("next", token + 1, p, None);
    assert_eq!(bad["kind"], "aborted");
    assert!(hold_token_alive(token));
    hunt_fight::on_reset();
    assert!(!hold_token_alive(token));
}

#[test]
fn timeout_at_tile_yields_without_rotate() {
    reset();
    hunt_fight::set_observation(obs_at(off_anchor()));
    let token = begin();
    let p = proj(json!({ "style": "range" }));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk");
    hunt_fight::set_observation(obs_at(safespot_a()));
    assert!(hold_force_bound_reached(token));
    let step = call(
        "next",
        token,
        p,
        Some(json!({ "queued": true, "walkToken": 18 })),
    );
    assert_eq!(step["kind"], "yield");
}

#[test]
fn retreat_and_walk_to_spot_classes_are_live() {
    let iso = LoadIsolate::spawn(
        r#"
import { HoldSafespot, Retreat, WalkToSpot } from '../../api/combat/hunting/combat.js';
export default class T extends LoopingBot {
    loop() {
        const probe = { retreat: null, walkToSpot: null, holdDue: typeof HoldSafespot };
        try { new Retreat(); } catch (e) { probe.retreat = String(e && e.message || e); }
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
        probe["retreat"].is_null() || !probe["retreat"].as_str().unwrap_or("").contains("not impl"),
        "Retreat isolate is live: {probe:?}"
    );
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
fn v1_execute_calls_interrupt_watch_and_queues_walk() {
    let iso = LoadIsolate::spawn(
        r#"
import { HoldSafespot } from '../../api/combat/hunting/combat.js';
export default class T extends LoopingBot {
    loop() {
        const host = {
            died: false,
            targetIdx: null,
            hpFraction: () => 1,
            panicHp: () => 0.2,
            retreatHp: () => 0.5,
            hasFood: () => true,
            needEat: () => false,
            style: () => 'melee',
            safespotIndex: () => 0,
            buryBones: () => false,
            boneName: () => 'Bones',
            fight: { interruptWatch() { globalThis.__iw = true; } },
            log() {},
            setStatus(m) { globalThis.__status = m; },
            setSafespotIndex() {},
        };
        const site = {
            key: 'test',
            target: 'Goblin',
            alsoHunt: [],
            safespots: [{ x: 2901, z: 9809, level: 0 }],
            meleeAnchor: { x: 2900, z: 9808, level: 0 },
            boxes: [{ minX: 2888, maxX: 2923, minZ: 9769, maxZ: 9816, level: 0 }],
            fireAtRange: false,
            rangedThreat: false,
        };
        const hold = new HoldSafespot(host, site);
        globalThis.__token = hold.token;
        globalThis.__hostFight = host.fight === hold;
        hold.execute();
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
    assert_eq!(host_fight, false, "Hold must not assign host.fight");
    let walks: Vec<_> = drained
        .into_iter()
        .filter(|req| matches!(req, InteractReq::Walk { .. }))
        .collect();
    assert!(
        walks.iter().any(|req| matches!(
            req,
            InteractReq::Walk {
                x: 2900,
                z: 9808,
                level: 0,
                allow_teleports: false,
                allow_wilderness: true,
                allow_bank_fetch: true,
                request_id,
            } if *request_id != 0
        )),
        "{walks:?}"
    );
}

#[test]
fn v1_in_area_predicate_without_boxes_is_asked() {
    let iso = LoadIsolate::spawn(
        r#"
import { Fight } from '../../api/combat/hunting/combat.js';
export default class T extends LoopingBot {
    loop() {
        const host = {
            died: true,
            hpFraction: () => 1,
            panicHp: () => 0.1,
            retreatHp: () => 0.2,
            hasFood: () => true,
            needEat: () => false,
            style: () => 'melee',
            safespotIndex: () => 0,
            buryBones: () => false,
            boneName: () => 'Bones',
            log() {},
            setStatus() {},
        };
        const site = {
            key: 't',
            target: 'Goblin',
            alsoHunt: [],
            safespots: [{ x: 2900, z: 9808, level: 0 }],
            meleeAnchor: { x: 2900, z: 9808, level: 0 },
            inArea(t) {
                globalThis.__area = (globalThis.__area || 0) + 1;
                return t.x === 2900 && t.z === 9808;
            },
        };
        const fight = globalThis.__f || (globalThis.__f = new Fight(host, site));
        globalThis.__inside = fight.validate();
        fight.execute();
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
            x: 2900,
            z: 9808,
            level: 0,
        },
    )));
    iso.on_game_tick(1);
    let area = iso.probe("globalThis.__area").unwrap();
    iso.join();
    assert!(
        area.as_i64().unwrap_or(0) >= 1,
        "inArea must be asked when the site has no boxes: {area:?}"
    );
}

#[test]
fn wait_fed_settles_true_and_false() {
    let iso = LoadIsolate::spawn(
        r#"
import { waitFed } from '../../api/combat/hunting/supply.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__ran) return;
        globalThis.__ran = true;
        globalThis.__true = await waitFed(() => true, 0);
        globalThis.__false = await waitFed(() => false, 0);
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
            x: 3200,
            z: 3200,
            level: 0,
        },
    )));
    iso.on_game_tick(1);
    iso.on_game_tick(2);
    let yes = iso.probe("globalThis.__true").unwrap();
    let no = iso.probe("globalThis.__false").unwrap();
    iso.join();
    assert_eq!(yes, true, "{yes:?}");
    assert_eq!(no, false, "{no:?}");
}

#[test]
fn teleport_out_reports_escape_shortfall() {
    let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
    let iso = LoadIsolate::spawn_with_game_data(
        r#"
import { teleportOut } from '../../api/combat/hunting/supply.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__ran) return;
        globalThis.__ran = true;
        const host = { log(m) { globalThis.__log = m; }, setStatus() {} };
        const site = { escapeTeleportId: 'varrock', inArea: () => true };
        globalThis.__out = await teleportOut(host, site);
    }
}
"#
        .to_string(),
        LoadShape::CompatClass,
        vec![],
        data,
    )
    .unwrap();
    let mut snap = empty_snapshot(
        1,
        TileInput {
            x: 3200,
            z: 3200,
            level: 0,
        },
    );
    let stats = [StatInput {
        index: 6,
        name: "Magic",
        xp: 0,
        base: 1,
        effective: 1,
    }];
    snap.stats = &stats;
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&snap));
    iso.on_game_tick(1);
    iso.on_game_tick(2);
    let out = iso.probe("globalThis.__out").unwrap();
    iso.join();
    let text = out.as_str().unwrap_or("");
    assert!(
        text.contains("magic 1 is below the") && text.contains("it needs"),
        "{out:?}"
    );
}

#[test]
fn acquire_key_state_is_held_when_carried_or_unneeded() {
    let iso = LoadIsolate::spawn(
        r#"
import { acquireKey } from '../../api/combat/hunting/supply.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__ran) return;
        globalThis.__ran = true;
        const host = { log() {}, setStatus() {} };
        globalThis.__none = await acquireKey(host, { key: 't', keyItem: null });
        globalThis.__held = await acquireKey(host, {
            key: 't',
            keyItem: { name: 'Dusty key', id: 1590 },
        });
    }
}
"#
        .to_string(),
        LoadShape::CompatClass,
        vec![],
    )
    .unwrap();
    let mut snap = empty_snapshot(
        1,
        TileInput {
            x: 3200,
            z: 3200,
            level: 0,
        },
    );
    let inv = [ItemRowInput {
        name: Some("Dusty key"),
        count: 1,
        id: 1590,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot: 0,
    }];
    snap.inv = &inv;
    iso.post_snapshot(script::isolate_fb::encode_snapshot(&snap));
    iso.on_game_tick(1);
    iso.on_game_tick(2);
    let none = iso.probe("globalThis.__none").unwrap();
    let held = iso.probe("globalThis.__held").unwrap();
    iso.join();
    assert_eq!(none, "held", "{none:?}");
    assert_eq!(held, "held", "{held:?}");
}

#[test]
fn acquire_key_state_is_fetch_when_the_key_is_missing() {
    let iso = LoadIsolate::spawn(
        r#"
import { acquireKey } from '../../api/combat/hunting/supply.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__ran) return;
        globalThis.__ran = true;
        const host = { log() {}, setStatus(m) { globalThis.__status = m; } };
        acquireKey(host, { key: 't', keyItem: { name: 'Dusty key', id: 1590 } }).then((v) => {
            globalThis.__state = v;
        });
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
            x: 3200,
            z: 3200,
            level: 0,
        },
    )));
    iso.on_game_tick(1);
    let status = iso.probe("globalThis.__status").unwrap();
    iso.join();
    assert!(
        status
            .as_str()
            .unwrap_or("")
            .contains("fetching the Dusty key"),
        "missing key banks then Velrak: {status:?}"
    );
}
