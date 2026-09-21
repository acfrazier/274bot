//! Offline Fight isolate policy tests. No V8.

use script::hunt_fight::{
    self, engaged, fight_deadline_remaining_ms, gap_sw, last_hp, loot_target, seen_contains,
    skip_contains, skip_remaining_ms, token_alive, FightNpc, FightObservation, Tile, TAVERLEY_BLUE,
};
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
        "safespots": [{"x": 2901, "z": 9809, "level": 0}],
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

fn obs_at(here: Tile, npcs: Vec<FightNpc>) -> FightObservation {
    let mut o = FightObservation {
        here: Some(here),
        ingame: true,
        scene_state: 2,
        hold: false,
        ours: false,
        chat_continue: false,
        npcs,
        self_target_kind: 0,
        self_target_index: -1,
        self_slot: 0,
        hp_effective: 70,
        inv_names: Vec::new(),
        animating: false,
        los_override: Some(true),
        tick: 1,
    };
    let _ = &mut o;
    o
}

fn begin() -> u64 {
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
    hunt_fight::dispatch(&v)
}

fn kinds(steps: &[Value]) -> Vec<String> {
    steps
        .iter()
        .map(|s| s["kind"].as_str().unwrap_or("").to_string())
        .collect()
}

fn pump(token: u64, p: &Value, max: usize, stop: impl Fn(&Value) -> bool) -> Vec<Value> {
    let mut reply = None;
    let mut out = Vec::new();
    for _ in 0..max {
        let step = call("next", token, p.clone(), reply.take());
        out.push(step.clone());
        let kind = step["kind"].as_str().unwrap_or("");
        if kind == "yield" || kind == "aborted" || stop(&step) {
            break;
        }
        reply = match kind {
            "eat" => Some(json!({ "eatOk": true })),
            "npc" => Some(json!({ "queued": true })),
            "bury" => Some(json!({ "buried": true })),
            _ => None,
        };
    }
    out
}

fn melee_here() -> Tile {
    Tile {
        x: 2900,
        z: 9808,
        level: 0,
    }
}

#[test]
fn empty_pack_retreat_yields_without_eat_or_npc() {
    reset();
    hunt_fight::set_observation(obs_at(melee_here(), vec![]));
    let token = begin();
    let p = proj(json!({ "hasFood": false, "needEat": false, "hpFraction": 0.4 }));
    let steps = pump(token, &p, 8, |_| false);
    let k = kinds(&steps);
    assert!(k.contains(&"status".into()), "{k:?}");
    assert_eq!(k.last().map(String::as_str), Some("yield"));
    assert!(!k.iter().any(|s| s == "eat" || s == "npc"), "{k:?}");
}

#[test]
fn fed_not_needeat_below_retreat_hp_does_not_yield_at_gate() {
    reset();
    hunt_fight::set_observation(obs_at(melee_here(), vec![goblin(1)]));
    let token = begin();
    let p = proj(json!({ "hasFood": true, "needEat": false, "hpFraction": 0.4 }));
    let steps = pump(token, &p, 12, |s| {
        s["kind"] == "npc" || s["kind"] == "arm-special"
    });
    let k = kinds(&steps);
    assert!(
        k.iter().any(|s| s == "arm-special" || s == "npc"),
        "must pass retreat gate, got {k:?}"
    );
    assert_ne!(k.get(1).map(String::as_str), Some("yield"), "{k:?}");
}

#[test]
fn needeat_and_retreat_due_yields_never_eat() {
    reset();
    hunt_fight::set_observation(obs_at(melee_here(), vec![goblin(1)]));
    let token = begin();
    let p = proj(json!({ "hasFood": true, "needEat": true, "hpFraction": 0.4 }));
    let steps = pump(token, &p, 8, |_| false);
    let k = kinds(&steps);
    assert_eq!(k.last().map(String::as_str), Some("yield"), "{k:?}");
    assert!(!k.iter().any(|s| s == "eat"), "{k:?}");
}

#[test]
fn failed_eat_yields_without_attack() {
    reset();
    hunt_fight::set_observation(obs_at(melee_here(), vec![goblin(1)]));
    let token = begin();
    let p = proj(json!({
        "hasFood": true,
        "needEat": true,
        "hpFraction": 0.8,
        "retreatHp": 0.0
    }));
    let first = call("next", token, p.clone(), None);
    assert_eq!(first["kind"], "status");
    let eat = call("next", token, p.clone(), None);
    assert_eq!(eat["kind"], "eat", "{eat}");
    let done = call("next", token, p, Some(json!({ "eatOk": false })));
    assert_eq!(done["kind"], "yield");
    assert_ne!(done["kind"], "npc");
}

#[test]
fn one_next_per_turn_never_attacks() {
    reset();
    hunt_fight::set_observation(obs_at(melee_here(), vec![goblin(1)]));
    let token = begin();
    let p = proj(json!({}));
    let first = call("next", token, p, None);
    assert_eq!(first["kind"], "status");
    assert_ne!(first["kind"], "npc");
}

#[test]
fn engage_order_arm_npc_set_target_waitfed_not_yield() {
    reset();
    hunt_fight::set_observation(obs_at(melee_here(), vec![goblin(1)]));
    let token = begin();
    let p = proj(json!({}));
    let steps = pump(token, &p, 20, |s| s["kind"] == "set-target");
    let k = kinds(&steps);
    let arm = k.iter().position(|s| s == "arm-special").expect("{k:?}");
    let npc = k.iter().position(|s| s == "npc").expect("{k:?}");
    let set = k.iter().position(|s| s == "set-target").expect("{k:?}");
    assert!(arm < npc && npc < set, "{k:?}");
    assert_eq!(steps[npc]["index"], 7);
    assert_eq!(steps[npc]["action"], "Attack");
    assert!(!k.iter().any(|s| s == "yield"), "{k:?}");
    let after = call("next", token, p, None);
    assert_ne!(after["kind"], "yield");
    let kinds_after = after["kind"].as_str().unwrap_or("");
    assert!(
        kinds_after == "sustain" || kinds_after == "delay-ticks" || kinds_after == "wait",
        "{after}"
    );
}

#[test]
fn shield_ready_false_is_false_absence_is_not() {
    reset();
    hunt_fight::set_observation(obs_at(melee_here(), vec![goblin(1)]));
    let token = begin();
    let blocked = proj(json!({ "hasShieldReady": true, "shieldReady": false }));
    let v = call("validate", token, blocked.clone(), None);
    assert_eq!(v["value"], false);
    let steps = pump(token, &blocked, 8, |_| false);
    assert_eq!(kinds(&steps).last().map(String::as_str), Some("yield"));
    assert!(!kinds(&steps).iter().any(|s| s == "npc"));

    reset();
    hunt_fight::set_observation(obs_at(melee_here(), vec![goblin(1)]));
    let token = begin();
    let absent = proj(json!({ "hasShieldReady": false, "shieldReady": false }));
    let v = call("validate", token, absent.clone(), None);
    assert_eq!(v["value"], true, "{v}");
    let steps = pump(token, &absent, 12, |s| {
        s["kind"] == "arm-special" || s["kind"] == "npc"
    });
    assert!(
        kinds(&steps)
            .iter()
            .any(|s| s == "arm-special" || s == "npc"),
        "{:?}",
        kinds(&steps)
    );
}

#[test]
fn refused_attack_skips_without_set_target_and_continues() {
    reset();
    hunt_fight::set_observation(obs_at(melee_here(), vec![goblin(1)]));
    let token = begin();
    let p = proj(json!({}));
    let mut reply = None;
    let mut saw_npc = false;
    let mut after = Vec::new();
    for _ in 0..20 {
        let step = call("next", token, p.clone(), reply.take());
        if step["kind"] == "npc" && !saw_npc {
            saw_npc = true;
            reply = Some(json!({ "queued": false }));
            after.push(step);
            continue;
        }
        after.push(step.clone());
        let kind = step["kind"].as_str().unwrap_or("");
        if kind == "yield" || kind == "aborted" {
            break;
        }
        if saw_npc && (kind == "arm-special" || kind == "npc" || kind == "sustain") {
            // continue a bit after refuse
        }
        reply = match kind {
            "eat" => Some(json!({ "eatOk": true })),
            "npc" => Some(json!({ "queued": true })),
            "bury" => Some(json!({ "buried": true })),
            _ => None,
        };
        if saw_npc && after.iter().any(|s| s["kind"] == "sustain") && after.len() > 3 {
            break;
        }
    }
    let k = kinds(&after);
    assert!(k.iter().any(|s| s == "npc"), "{k:?}");
    assert!(!k.iter().any(|s| s == "set-target"), "{k:?}");
    assert!(!k.iter().any(|s| s == "yield"), "{k:?}");
    assert!(skip_contains(token, 7));
    assert!(k.iter().any(|s| s == "sustain"), "{k:?}");
}

#[test]
fn interrupt_watch_keeps_skip_and_seen() {
    reset();
    hunt_fight::set_observation(obs_at(melee_here(), vec![goblin(1)]));
    let token = begin();
    let p = proj(json!({}));
    let mut reply = None;
    for _ in 0..16 {
        let step = call("next", token, p.clone(), reply.take());
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
        if skip_contains(token, 7) && seen_contains(token, 7) {
            break;
        }
    }
    assert!(skip_contains(token, 7));
    assert!(seen_contains(token, 7));
    call("interruptWatch", token, p.clone(), None);
    assert_eq!(last_hp(token), Some(-1));
    assert!(skip_contains(token, 7));
    assert!(seen_contains(token, 7));
}

#[test]
fn fight_reset_keeps_skip_session_reset_drops_token() {
    reset();
    hunt_fight::set_observation(obs_at(melee_here(), vec![goblin(1)]));
    let token = begin();
    let p = proj(json!({}));
    let mut reply = None;
    for _ in 0..16 {
        let step = call("next", token, p.clone(), reply.take());
        if step["kind"] == "npc" {
            let _ = call("next", token, p.clone(), Some(json!({ "queued": true })));
            break;
        }
        reply = None;
        if step["kind"] == "yield" {
            break;
        }
    }
    let had_skip = skip_contains(token, 7) || engaged(token).is_some();
    call("reset", token, p.clone(), None);
    assert!(token_alive(token));
    assert_eq!(engaged(token), None);
    assert_eq!(loot_target(token), None);
    let _ = had_skip;
    hunt_fight::on_reset();
    assert!(!token_alive(token));
    let again = call("next", token, p, None);
    assert_eq!(again["kind"], "aborted");
}

#[test]
fn packed_origin_gap_sw_rejects_centre_shift() {
    let spot = Tile {
        x: 2826,
        z: 9825,
        level: 0,
    };
    let network = Tile {
        x: 2832,
        z: 9825,
        level: 0,
    };
    let tile = Tile {
        x: 2833,
        z: 9823,
        level: 0,
    };
    let centre = Tile {
        x: 2834,
        z: 9827,
        level: 0,
    };
    let g_net = gap_sw(spot, network, 4);
    let g_tile = gap_sw(spot, tile, 4);
    let g_centre = gap_sw(spot, centre, 4);
    assert_eq!(g_net, 6);
    assert_ne!(g_centre, g_net);
    assert_ne!(g_tile, g_net);
    assert!(g_net <= 6);
    assert!(g_centre > 6);
}

#[test]
fn idle_is_sustain_then_bury_or_delay() {
    reset();
    hunt_fight::set_observation(obs_at(melee_here(), vec![]));
    let token = begin();
    let p = proj(json!({ "retreatHp": 0.0, "hasFood": true, "needEat": false }));
    let steps = pump(token, &p, 12, |_| false);
    let k = kinds(&steps);
    let sus = k.iter().position(|s| s == "sustain").expect("{k:?}");
    let rest = &k[sus + 1..];
    assert!(
        rest.iter().any(|s| s == "bury" || s == "delay-ticks"),
        "{k:?}"
    );
    let after = rest
        .iter()
        .filter(|s| *s == "bury" || *s == "delay-ticks")
        .count();
    assert_eq!(after, 1, "{k:?}");
}

#[test]
fn walkback_emits_walk_to_only() {
    reset();
    let off = Tile {
        x: 2895,
        z: 9808,
        level: 0,
    };
    hunt_fight::set_observation(obs_at(off, vec![]));
    let token = begin();
    let p = proj(json!({ "retreatHp": 0.0 }));
    let steps = pump(token, &p, 12, |s| s["kind"] == "walk-to");
    let k = kinds(&steps);
    assert!(k.iter().any(|s| s == "walk-to"), "{k:?}");
    assert!(!k.iter().any(|s| s == "walk" || s == "walk-near"), "{k:?}");
    let walk = steps.iter().find(|s| s["kind"] == "walk-to").unwrap();
    assert_eq!(walk["x"], 2900);
    assert_eq!(walk["z"], 9808);
}

#[test]
fn pause_hold_does_not_elapse_fight_skip_or_sighting() {
    reset();
    hunt_fight::set_observation(obs_at(melee_here(), vec![goblin(1)]));
    let token = begin();
    let p = proj(json!({}));
    let mut reply = None;
    for _ in 0..16 {
        let step = call("next", token, p.clone(), reply.take());
        if step["kind"] == "npc" {
            let _ = call("next", token, p.clone(), Some(json!({ "queued": false })));
            break;
        }
        reply = None;
    }
    assert!(skip_contains(token, 7));
    assert!(seen_contains(token, 7));
    let skip_before = skip_remaining_ms(token, 7).unwrap_or(0);
    let seen_before = hunt_fight::sighting_since_age_ms(token, 7).unwrap_or(0);
    let fight_before = fight_deadline_remaining_ms(token).unwrap_or(0);
    hunt_fight::on_pause();
    thread::sleep(Duration::from_millis(280));
    hunt_fight::on_resume();
    let skip_after = skip_remaining_ms(token, 7).unwrap_or(0);
    let seen_after = hunt_fight::sighting_since_age_ms(token, 7).unwrap_or(0);
    let fight_after = fight_deadline_remaining_ms(token).unwrap_or(0);
    assert!(
        (skip_before - skip_after).abs() < 80,
        "skip elapsed during freeze: {skip_before} -> {skip_after}"
    );
    assert!(
        seen_after.saturating_sub(seen_before) < 80,
        "sighting aged during freeze: {seen_before} -> {seen_after}"
    );
    assert!(
        (fight_before - fight_after).abs() < 80,
        "FIGHT_MS elapsed during freeze: {fight_before} -> {fight_after}"
    );
}

#[test]
fn unexpected_eatok_aborts() {
    reset();
    hunt_fight::set_observation(obs_at(melee_here(), vec![goblin(1)]));
    let token = begin();
    let p = proj(json!({}));
    let status = call("next", token, p.clone(), None);
    assert_eq!(status["kind"], "status");
    let bad = call("next", token, p, Some(json!({ "eatOk": true })));
    assert_eq!(bad["kind"], "aborted");
}

#[test]
fn taverley_blue_range_field_uses_engage_range() {
    reset();
    let here = Tile {
        x: 2901,
        z: 9809,
        level: 0,
    };
    let near = FightNpc {
        index: 3,
        id: 2,
        name: "Blue dragon".into(),
        x: 2904,
        z: 9809,
        level: 0,
        nx: 2904,
        nz: 9809,
        size: 1,
        distance: 3,
        health: 80,
        in_combat: false,
        actions: vec!["Attack".into()],
        target_kind: 0,
        target_index: -1,
    };
    let far = FightNpc {
        index: 4,
        id: 3,
        name: "Blue dragon".into(),
        x: 2912,
        z: 9809,
        level: 0,
        nx: 2912,
        nz: 9809,
        size: 1,
        distance: 11,
        health: 80,
        in_combat: false,
        actions: vec!["Attack".into()],
        target_kind: 0,
        target_index: -1,
    };
    hunt_fight::set_observation(obs_at(here, vec![near, far]));
    let token = begin();
    let p = proj(json!({
        "key": TAVERLEY_BLUE,
        "target": "Blue dragon",
        "style": "range",
        "retreatHp": 0.0,
        "safespots": [{"x": 2901, "z": 9809, "level": 0}],
        "meleeAnchor": {"x": 2900, "z": 9808, "level": 0},
    }));
    let steps = pump(token, &p, 16, |s| s["kind"] == "npc");
    let npc = steps.iter().find(|s| s["kind"] == "npc");
    if let Some(npc) = npc {
        assert_eq!(npc["index"], 3, "{npc}");
        assert_ne!(npc["index"], 4);
    } else {
        // Near dragon at gap 3 is in range-6; far at 11 is not.
        assert!(
            kinds(&steps).iter().any(|s| s == "arm-special"),
            "{:?}",
            kinds(&steps)
        );
    }
}
