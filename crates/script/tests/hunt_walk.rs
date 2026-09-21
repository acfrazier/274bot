//! Offline WalkToSpot isolate policy tests. No LIVE.

use script::hunt_fight::{
    self, last_hp, walk_deadline_remaining_ms, walk_force_bound_reached, walk_token_alive,
    FightObservation, Tile,
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
            {"x": 2910, "z": 9809, "level": 0},
            {"x": 2901, "z": 9809, "level": 0}
        ],
        "meleeAnchor": {"x": 2900, "z": 9808, "level": 0},
        "boxes": [{"minX": 2888, "maxX": 2923, "minZ": 9769, "maxZ": 9816, "level": 0}],
        "fireAtRange": false,
        "rangedThreat": false,
        "approach": [],
    })
}

fn proj(extra: Value) -> Value {
    let mut v = json!({
        "died": false,
        "targetIdx": null,
        "hpFraction": 1.0,
        "panicHp": 0.2,
        "retreatHp": 0.5,
        "hasFood": false,
        "needEat": false,
        "style": "range",
        "safespotIndex": 1,
        "buryBones": false,
        "boneName": "Bones",
        "hasVlog": false,
        "hasArmSpecial": false,
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

fn obs_at(here: Option<Tile>) -> FightObservation {
    FightObservation {
        here,
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

fn tile(x: i32, z: i32, level: i32) -> Tile {
    Tile { x, z, level }
}

fn far() -> Tile {
    tile(2914, 9809, 0)
}

fn indexed_spot() -> Tile {
    tile(2901, 9809, 0)
}

fn melee_anchor() -> Tile {
    tile(2900, 9808, 0)
}

fn begin() -> u64 {
    hunt_fight::walk_dispatch(&json!({ "op": "begin" }))["token"]
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
    hunt_fight::walk_dispatch(&v)
}

fn validate(token: u64, p: Value) -> bool {
    call("validate", token, p, None)["value"]
        .as_bool()
        .unwrap_or(false)
}

fn ack(token: u64) -> Value {
    json!({ "queued": true, "walkToken": token })
}

#[test]
fn far_validates_near_and_hold_window_do_not() {
    reset();
    hunt_fight::set_observation(obs_at(Some(far())));
    let token = begin();
    let p = proj(json!({}));
    assert!(validate(token, p.clone()), "chebyshev 13");
    hunt_fight::set_observation(obs_at(Some(tile(2913, 9809, 0))));
    assert!(
        !validate(token, p.clone()),
        "chebyshev 12 is Hold, not this task"
    );
    hunt_fight::set_observation(obs_at(Some(indexed_spot())));
    assert!(!validate(token, p.clone()), "atTile is not a long walk-in");
    hunt_fight::set_observation(obs_at(Some(tile(2906, 9809, 0))));
    assert!(
        !validate(token, proj(json!({ "hasFood": true }))),
        "tiles 1-12 stay false even when holdDue would be true"
    );
    hunt_fight::set_observation(obs_at(Some(far())));
    assert!(
        validate(token, proj(json!({ "hasFood": false }))),
        "far walk-in does not consult holdDue"
    );
}

#[test]
fn chase_area_panic_and_level_gates() {
    reset();
    hunt_fight::set_observation(obs_at(Some(far())));
    let token = begin();
    assert!(
        !validate(
            token,
            proj(json!({ "style": "melee", "fireAtRange": true, "targetIdx": 4 }))
        ),
        "live chase blocks the walk-in"
    );
    assert!(
        validate(
            token,
            proj(json!({ "style": "melee", "fireAtRange": true, "targetIdx": null }))
        ),
        "chase mode without a target still walks in"
    );
    assert!(!validate(token, proj(json!({ "hpFraction": 0.1 }))));
    assert!(validate(token, proj(json!({ "hpFraction": 0.2 }))));
    hunt_fight::set_observation(obs_at(Some(tile(2800, 9809, 0))));
    assert!(!validate(token, proj(json!({}))));
    hunt_fight::set_observation(obs_at(None));
    assert!(!validate(token, proj(json!({}))));
    hunt_fight::set_observation(obs_at(Some(far())));
    assert!(
        validate(
            token,
            proj(json!({
                "safespots": [{"x": 2901, "z": 9809, "level": 1}],
            }))
        ),
        "different level is distanceTo >= 1_000_000"
    );
}

#[test]
fn dest_is_existing_anchor_not_retreat_aim() {
    reset();
    hunt_fight::set_observation(obs_at(Some(far())));
    let token = begin();
    let p = proj(json!({}));
    let status = call("next", token, p.clone(), None);
    assert_eq!(status["kind"], "status");
    assert_ne!(status["kind"], "walk-to");
    let step = call("next", token, p, None);
    assert_eq!(step["kind"], "walk");
    assert_eq!(step["x"], 2901);
    assert_eq!(step["z"], 9809);
    assert_ne!(step["x"], 2900, "not meleeAnchor");
    assert_ne!(step["x"], 2910, "not nearest safespot / retreatAim");

    reset();
    hunt_fight::set_observation(obs_at(Some(tile(2914, 9808, 0))));
    let melee = begin();
    let mp = proj(json!({ "style": "melee", "safespotIndex": 0 }));
    assert_eq!(call("next", melee, mp.clone(), None)["kind"], "status");
    let walk = call("next", melee, mp, None);
    assert_eq!(walk["kind"], "walk");
    assert_eq!(walk["x"], 2900);
    assert_eq!(walk["z"], 9808);
    assert_eq!(walk["level"], 0);

    reset();
    hunt_fight::set_observation(obs_at(Some(far())));
    let empty = begin();
    let ep = proj(json!({ "safespots": [], "style": "range" }));
    assert_eq!(call("next", empty, ep.clone(), None)["kind"], "status");
    let fallback = call("next", empty, ep, None);
    assert_eq!(fallback["kind"], "walk");
    assert_eq!(fallback["x"], melee_anchor().x);
    assert_eq!(fallback["z"], melee_anchor().z);

    reset();
    hunt_fight::set_observation(obs_at(Some(far())));
    let neg = begin();
    let np = proj(json!({ "safespotIndex": -1, "style": "range" }));
    assert_eq!(call("next", neg, np.clone(), None)["kind"], "status");
    let clamped = call("next", neg, np, None);
    assert_eq!(
        clamped["x"], 2910,
        "do not edit anchor(); negative index stays safespots[0]"
    );
    assert_ne!(clamped["x"], melee_anchor().x);
}

#[test]
fn empty_approach_walks_dest_only_and_nearest_skips_within_one() {
    reset();
    hunt_fight::set_observation(obs_at(Some(far())));
    let token = begin();
    let p = proj(json!({ "approach": [] }));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert!(walk_deadline_remaining_ms(token).is_none());
    let walk = call("next", token, p, None);
    assert_eq!(walk["kind"], "walk");
    assert_eq!(walk["x"], indexed_spot().x);
    let left = walk_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        left > 100_000,
        "armed APPROACH_MS, not RETURN_MS/HOP_MS: {left}"
    );

    reset();
    hunt_fight::set_observation(obs_at(Some(tile(101, 0, 0))));
    let near = begin();
    let stops = proj(json!({
        "approach": [
            {"x": 100, "z": 0, "level": 0},
            {"x": 200, "z": 0, "level": 0}
        ],
        "boxes": [{"minX": 0, "maxX": 400, "minZ": 0, "maxZ": 40, "level": 0}],
        "safespots": [{"x": 2901, "z": 9809, "level": 0}],
        "safespotIndex": 0,
    }));
    assert_eq!(call("next", near, stops.clone(), None)["kind"], "status");
    let first = call("next", near, stops.clone(), None);
    assert_eq!(first["kind"], "walk");
    assert_eq!(first["x"], 200);
    assert_eq!(first["z"], 0);
    assert!(walk_force_bound_reached(near));
    let dest = call("next", near, stops, Some(ack(41)));
    assert_eq!(
        dest["kind"], "walk",
        "failed approach leg advances, no short-stop log"
    );
    assert_eq!(dest["x"], 2901);
    let rearmed = walk_deadline_remaining_ms(near).unwrap_or(0);
    assert!(
        rearmed > 100_000,
        "one deadline re-armed for the dest leg: {rearmed}"
    );
}

#[test]
fn approach_starts_at_nearest_and_level_mismatch_does_not_skip() {
    reset();
    hunt_fight::set_observation(obs_at(Some(tile(16, 0, 0))));
    let token = begin();
    let p = proj(json!({
        "approach": [
            {"x": 0, "z": 0, "level": 0},
            {"x": 18, "z": 0, "level": 0}
        ],
        "boxes": [{"minX": 0, "maxX": 40, "minZ": 0, "maxZ": 40, "level": 0}],
    }));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    let first = call("next", token, p, None);
    assert_eq!(
        (first["kind"].as_str(), first["x"].as_i64()),
        (Some("walk"), Some(18))
    );

    reset();
    hunt_fight::set_observation(obs_at(Some(tile(100, 0, 0))));
    let other = begin();
    let level = proj(json!({
        "approach": [{"x": 100, "z": 0, "level": 1}],
        "boxes": [{"minX": 0, "maxX": 200, "minZ": 0, "maxZ": 20, "level": 0}],
        "safespots": [{"x": 50, "z": 0, "level": 0}],
        "safespotIndex": 0,
    }));
    assert_eq!(call("next", other, level.clone(), None)["kind"], "status");
    let stop = call("next", other, level, None);
    assert_eq!(stop["kind"], "walk");
    assert_eq!(stop["level"], 1, "level mismatch must not use xz-only skip");
}

#[test]
fn leftover_position_does_not_settle_before_this_walk() {
    reset();
    hunt_fight::set_observation(obs_at(Some(indexed_spot())));
    let token = begin();
    let p = proj(json!({}));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    let walk = call("next", token, p.clone(), None);
    assert_eq!(
        walk["kind"], "walk",
        "here==dest before emit is not Traveller success"
    );
    let settled = call("next", token, p, Some(ack(7)));
    assert_eq!(settled["kind"], "yield");
    assert_ne!(settled["kind"], "log");
}

#[test]
fn unmatched_walk_token_does_not_settle_and_at_tile_after_wait_skips_log() {
    reset();
    hunt_fight::set_observation(obs_at(Some(far())));
    let token = begin();
    let p = proj(json!({}));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk");
    let waiting = call("next", token, p.clone(), Some(ack(99)));
    assert_eq!(
        waiting["kind"], "sustain",
        "stale/other request id must not settle"
    );
    assert_ne!(waiting["kind"], "yield");
    assert_ne!(waiting["kind"], "wait-fed-done");
    let delay = call("next", token, p.clone(), None);
    assert_eq!(delay["kind"], "delay-ticks");
    assert_eq!(delay["n"], 1);

    hunt_fight::set_observation(obs_at(Some(indexed_spot())));
    assert!(walk_force_bound_reached(token));
    let arrived = call("next", token, p, None);
    assert_eq!(
        arrived["kind"], "yield",
        "re-read atTile before the log; true yields even if walk_wait is false"
    );
}

#[test]
fn short_stop_logs_then_yields_without_rotate() {
    reset();
    hunt_fight::set_observation(obs_at(Some(far())));
    let token = begin();
    let p = proj(json!({ "safespotIndex": 1 }));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk");
    assert!(walk_force_bound_reached(token));
    let log = call("next", token, p.clone(), Some(ack(3)));
    assert_eq!(log["kind"], "log");
    assert_eq!(
        log["message"],
        "the walk in stopped short of safespot 1 at (2901, 9809, 0). Closing the gap from the walk-back task."
    );
    assert_ne!(log["kind"], "set-safespot");
    let done = call("next", token, p, None);
    assert_eq!(done["kind"], "yield");
}

#[test]
fn signal_stops_further_legs_then_logs_unless_at_dest() {
    reset();
    hunt_fight::set_observation(obs_at(Some(tile(16, 0, 0))));
    let token = begin();
    let p = proj(json!({
        "approach": [
            {"x": 0, "z": 0, "level": 0},
            {"x": 18, "z": 0, "level": 0}
        ],
        "boxes": [{"minX": 0, "maxX": 40, "minZ": 0, "maxZ": 40, "level": 0}],
        "safespots": [{"x": 2901, "z": 9809, "level": 0}],
        "safespotIndex": 0,
    }));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    let first = call("next", token, p.clone(), None);
    assert_eq!(first["x"], 18);
    let mut held = obs_at(Some(tile(16, 0, 0)));
    held.ours = true;
    hunt_fight::set_observation(held);
    let log = call("next", token, p.clone(), Some(ack(8)));
    assert_eq!(log["kind"], "log");
    assert!(log["message"]
        .as_str()
        .unwrap_or("")
        .contains("stopped short"));
    assert_eq!(call("next", token, p, None)["kind"], "yield");

    reset();
    hunt_fight::set_observation(obs_at(Some(indexed_spot())));
    let on_dest = begin();
    let dp = proj(json!({}));
    assert_eq!(call("next", on_dest, dp.clone(), None)["kind"], "status");
    let mut signal = obs_at(Some(indexed_spot()));
    signal.hold = true;
    hunt_fight::set_observation(signal);
    let step = call("next", on_dest, dp, None);
    assert_eq!(step["kind"], "yield", "atTile before the log suppresses it");
    assert_ne!(step["kind"], "walk");
    assert_ne!(step["kind"], "log");
}

#[test]
fn pause_and_hold_freeze_the_one_approach_deadline() {
    reset();
    hunt_fight::set_observation(obs_at(Some(far())));
    let token = begin();
    let p = proj(json!({}));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk");
    let before = walk_deadline_remaining_ms(token).unwrap_or(0);
    assert!(before > 100_000, "{before}");
    hunt_fight::on_pause();
    thread::sleep(Duration::from_millis(280));
    let frozen = call("next", token, p.clone(), Some(ack(5)));
    assert_eq!(frozen["kind"], "wait");
    hunt_fight::on_resume();
    let after = walk_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        (before - after).abs() < 80,
        "APPROACH_MS elapsed during pause: {before} -> {after}"
    );
    hunt_fight::on_hold(true);
    thread::sleep(Duration::from_millis(280));
    hunt_fight::on_hold(false);
    let held = walk_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        (after - held).abs() < 80,
        "APPROACH_MS elapsed during hold: {after} -> {held}"
    );
}

#[test]
fn siblings_keep_verbs_and_walk_token_is_separate() {
    reset();
    hunt_fight::set_observation(obs_at(Some(far())));
    let walk = begin();
    let fight = hunt_fight::dispatch(&json!({ "op": "begin" }))["token"]
        .as_u64()
        .unwrap();
    let hold = hunt_fight::hold_dispatch(&json!({ "op": "begin" }))["token"]
        .as_u64()
        .unwrap();
    let retreat = hunt_fight::retreat_dispatch(&json!({ "op": "begin" }))["token"]
        .as_u64()
        .unwrap();
    assert_ne!(walk, fight);
    assert_ne!(walk, hold);
    assert_ne!(walk, retreat);
    hunt_fight::dispatch(&json!({ "op": "interruptWatch", "token": fight }));
    assert_eq!(last_hp(fight), Some(-1));
    assert!(hunt_fight::token_alive(fight));

    let fp = proj(json!({ "approach": [{"x": 1, "z": 1, "level": 0}] }));
    let fight_step = hunt_fight::dispatch(&{
        let mut v = fp.clone();
        v["op"] = json!("next");
        v["token"] = json!(fight);
        v
    });
    assert_ne!(fight_step["kind"], "walk");

    let mut hp = proj(json!({ "style": "melee", "hasFood": true }));
    hp["op"] = json!("next");
    hp["token"] = json!(hold);
    let hold_step = hunt_fight::hold_dispatch(&hp);
    assert_ne!(hold_step["kind"], "walk-to");

    let mut rp = proj(json!({
        "style": "melee",
        "hasFood": false,
        "hpFraction": 0.4,
        "retreatHp": 0.5,
        "safespotIndex": 0,
    }));
    rp["op"] = json!("next");
    rp["token"] = json!(retreat);
    let mut saw_walk = false;
    let mut saw_walk_to = false;
    for _ in 0..6 {
        let step = hunt_fight::retreat_dispatch(&rp);
        let kind = step["kind"].as_str().unwrap_or("");
        saw_walk |= kind == "walk";
        saw_walk_to |= kind == "walk-to";
        if kind == "walk-to" || kind == "yield" || kind == "aborted" {
            break;
        }
    }
    assert!(!saw_walk);
    assert!(saw_walk_to, "Retreat still emits walk-to");

    let bad = call("next", walk + 50, proj(json!({})), None);
    assert_eq!(bad["kind"], "aborted");
    assert!(walk_token_alive(walk));
    hunt_fight::on_reset();
    assert!(!walk_token_alive(walk));
}

#[test]
fn unexpected_replies_abort_and_unknown_op_is_not_invented() {
    reset();
    hunt_fight::set_observation(obs_at(Some(far())));
    let token = begin();
    let p = proj(json!({}));
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    let eat = call("next", token, p.clone(), Some(json!({ "eatOk": true })));
    assert_eq!(eat["kind"], "aborted");

    let token = begin();
    assert_eq!(call("next", token, p.clone(), None)["kind"], "status");
    assert_eq!(call("next", token, p.clone(), None)["kind"], "walk");
    let npc = call("next", token, p.clone(), Some(json!({ "queued": true })));
    assert_eq!(npc["kind"], "aborted");
    let missing = hunt_fight::walk_dispatch(&json!({ "op": "walk-resilient", "token": token }));
    assert_eq!(missing["kind"], "notImpl");
}

#[test]
fn shim_dispatches_walk_and_calls_interrupt_watch() {
    let src = include_str!("../src/shim/hunting_combat.js");
    let walk = src
        .split("export class WalkToSpot")
        .nth(1)
        .expect("WalkToSpot");
    assert!(walk.contains("this.host.fight?.interruptWatch()"));
    assert!(walk.contains("op: 'walk'"));
    assert!(walk.contains("radius: 0"));
    assert!(walk.contains("walkspotCall"));
    assert!(src.contains("__rs2b0t_walkspot"));
    assert!(!walk.contains("walk-to"));
    assert!(!walk.contains("set-safespot"));
    assert!(!walk.contains("setSafespot"));
    assert!(!walk.contains("holdDue"));
    assert!(!walk.contains("anchorFor"));
    assert!(!walk.contains("Traversal"));
    assert!(!walk.contains("walkResilient"));
    assert!(!walk.contains("walkWorld"));
    assert!(!walk.contains("wait-fed-done"));
    assert!(src.contains("approach: (site.approach || []).map(tile)"));
    let fight = src.split("export class HoldSafespot").next().unwrap();
    assert!(fight.contains("case 'walk-to':"));
    assert!(!fight.contains("case 'walk':"));

    let iso = LoadIsolate::spawn(
        r#"
import { WalkToSpot } from '../../api/combat/hunting/combat.js';
export default class T extends LoopingBot {
    loop() {
        const host = {
            died: false,
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
            fight: { interruptWatch() { globalThis.__iw = true; } },
            log() {},
            setStatus(m) { globalThis.__status = m; },
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
            approach: [],
        };
        const walk = new WalkToSpot(host, site);
        globalThis.__token = walk.token;
        globalThis.__hostFight = host.fight === walk;
        walk.execute();
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
            x: 2914,
            z: 9809,
            level: 0,
        },
    )));
    iso.on_game_tick(1);
    let iw = iso.probe("globalThis.__iw").unwrap();
    let host_fight = iso.probe("globalThis.__hostFight").unwrap();
    let status = iso.probe("globalThis.__status").unwrap();
    let drained = iso.drain_interacts();
    iso.join();
    assert_eq!(
        iw, true,
        "execute must call host.fight.interruptWatch first"
    );
    assert_eq!(host_fight, false, "WalkToSpot must not assign host.fight");
    assert_eq!(status, "walking to the fight spot");
    assert!(
        drained
            .iter()
            .all(|req| !matches!(req, InteractReq::WalkTo { .. })),
        "{drained:?}"
    );
    assert!(
        drained
            .iter()
            .all(|req| !matches!(req, InteractReq::WalkNear { .. })),
        "{drained:?}"
    );
    assert!(
        drained.iter().any(|req| matches!(
            req,
            InteractReq::Walk {
                x: 2901,
                z: 9809,
                level: 0,
                allow_teleports: false,
                allow_wilderness: true,
                allow_bank_fetch: true,
                request_id,
            } if *request_id != 0
        )),
        "{drained:?}"
    );
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
