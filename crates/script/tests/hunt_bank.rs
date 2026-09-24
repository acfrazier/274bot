//! Offline Bank isolate policy tests. No LIVE.
//!
//! These lock the architecture corrections: slot -1 is not fetch, a failed
//! bank withdraw does not count the trip, the yield shape, no deposit-1, and
//! walk-near radius 3 with the three allow flags false. They do not mirror
//! phases.

use script::hunt_bank::{
    self, bank_deadline_remaining_ms, bank_force_bound_reached, bank_token_alive, set_observation,
    BankObservation, BankRow, APPROACH_RADIUS, CLOSE_MS, OPEN_MS, WALK_LEG_MS, WITHDRAW_MS,
};
use script::hunt_fight::Tile;
use script::isolate_fb::{encode_snapshot, ItemRowInput, SnapshotInput, SnapshotReader, TileInput};
use script::{LoadIsolate, LoadShape};
use serde_json::{json, Value};

fn reset() {
    hunt_bank::on_reset();
}

fn tile(x: i32, z: i32, level: i32) -> Tile {
    Tile { x, z, level }
}

fn row(id: i32, count: i32, name: &str, slot: i32, has_slot: bool) -> BankRow {
    BankRow {
        id,
        count,
        name: name.to_string(),
        slot,
        has_slot,
        ops: vec![],
    }
}

fn wearable(id: i32, name: &str, slot: i32) -> BankRow {
    let mut item = row(id, 1, name, slot, true);
    item.ops = vec!["Wear".into()];
    item
}

fn at(here: Tile) -> BankObservation {
    let mut obs = BankObservation::empty();
    obs.here = Some(here);
    obs.ingame = true;
    obs.hp_base = 99;
    obs.hp_effective = 99;
    obs.inv_size = 28;
    obs
}

fn boxes() -> Value {
    json!([{ "minX": 40, "maxX": 60, "minZ": 40, "maxZ": 60, "level": 0 }])
}

fn site(extra: Value) -> Value {
    let mut base = json!({
        "key": "taverley-blue",
        "keyItem": { "id": 1590, "name": "Dusty key" },
        "boxes": boxes(),
        "bank": { "x": 2946, "z": 3369, "level": 0 },
        "foodName": "Shark",
        "style": "range",
    });
    if let Some(obj) = extra.as_object() {
        for (k, v) in obj {
            base[k] = v.clone();
        }
    }
    base
}

fn begin() -> u64 {
    hunt_bank::dispatch(&json!({ "op": "begin" }))["token"]
        .as_u64()
        .expect("token")
}

fn call(token: u64, mut proj: Value, reply: Option<Value>) -> Value {
    proj["op"] = json!("next");
    proj["token"] = json!(token);
    if let Some(reply) = reply {
        proj["reply"] = reply;
    }
    hunt_bank::dispatch(&proj)
}

fn kind(step: &Value) -> &str {
    step["kind"].as_str().unwrap_or("")
}

fn assert_yield(step: &Value, value: bool) {
    assert_eq!(kind(step), "yield", "{step}");
    assert_eq!(step["value"], value, "{step}");
}

fn assert_not_forbidden(step: &Value) {
    for bad in [
        "npc",
        "obj",
        "loc",
        "use-on",
        "teleport",
        "attack",
        "eat",
        "withdraw-by-id",
        "deposit-1",
        "walk-nearest-bank",
    ] {
        assert_ne!(kind(step), bad, "{step}");
    }
}

fn open_loaded(mut obs: BankObservation) -> BankObservation {
    obs.bank_open = true;
    obs.bank_loaded = true;
    obs.bank_generation = 4;
    obs.bank_op_result_seq = 1;
    obs
}

fn booth() -> Tile {
    tile(2946, 3369, 0)
}

fn reach_open(token: u64, proj: &Value, obs: BankObservation) -> Value {
    set_observation(obs.clone());
    let step = call(token, proj.clone(), None);
    assert_eq!(kind(&step), "bank-open", "already at the booth: {step}");
    assert_not_forbidden(&step);
    set_observation(open_loaded(obs));
    let step = call(token, proj.clone(), Some(json!({ "opened": true })));
    assert_not_forbidden(&step);
    step
}

#[test]
fn leave_false_does_not_deposit_or_walk() {
    reset();
    set_observation(at(tile(50, 50, 0)));
    let token = begin();
    let proj = site(json!({}));
    let step = call(token, proj.clone(), None);
    assert_eq!(kind(&step), "leave", "{step}");
    assert_ne!(kind(&step), "walk-near");
    assert_ne!(kind(&step), "bank-open");
    let failed = call(token, proj, Some(json!({ "left": false })));
    assert_yield(&failed, false);
    assert_ne!(kind(&failed), "deposit");
    assert_ne!(kind(&failed), "count-bank-trip");
}

#[test]
fn walk_near_is_radius_3_with_flags_false_and_arrival_is_not_yield() {
    reset();
    set_observation(at(tile(1000, 1000, 0)));
    let token = begin();
    let proj = site(json!({
        "key": "gutanoth-blue",
        "keyItem": null,
        "bank": { "x": 2612, "z": 3092, "level": 0 },
    }));
    let step = call(token, proj.clone(), None);
    assert_eq!(kind(&step), "walk-near", "{step}");
    assert_eq!(step["x"], 2612);
    assert_eq!(step["z"], 3092);
    assert_eq!(step["level"], 0);
    assert_eq!(step["radius"], APPROACH_RADIUS);
    assert_eq!(step["radius"], 3);
    assert_eq!(step["allow_teleports"], false);
    assert_eq!(step["allow_wilderness"], false);
    assert_eq!(step["allow_bank_fetch"], false);
    assert_ne!(kind(&step), "walk-nearest-bank");
    let remaining = bank_deadline_remaining_ms(token).expect("walk window");
    assert!(remaining > 120_000, "walk leg is not 120s: {remaining}");
    assert!(remaining <= WALK_LEG_MS as i64, "{remaining}");
    let mut arrived = at(tile(2612, 3092, 0));
    arrived.bank_open = false;
    set_observation(arrived);
    let next = call(token, proj, Some(json!({ "queued": true, "walkToken": 9 })));
    assert_eq!(kind(&next), "bank-open", "arrival is not the pack: {next}");
    assert_ne!(kind(&next), "yield");
}

#[test]
fn opened_true_is_not_pack_ready() {
    reset();
    set_observation(at(booth()));
    let token = begin();
    let proj = site(json!({ "keyItem": null }));
    let step = call(token, proj.clone(), None);
    assert_eq!(kind(&step), "bank-open", "{step}");
    let waiting = call(token, proj, Some(json!({ "opened": true })));
    assert_eq!(kind(&waiting), "delay-ticks", "{waiting}");
    assert_ne!(kind(&waiting), "yield");
    assert_ne!(kind(&waiting), "deposit");
    assert_ne!(kind(&waiting), "count-bank-trip");
    let remaining = bank_deadline_remaining_ms(token).expect("open window");
    assert!(remaining <= OPEN_MS as i64, "{remaining}");
    assert!(remaining > 0, "{remaining}");
}

#[test]
fn slot_minus_one_is_not_fetch_or_a_withdraw() {
    reset();
    let mut obs = at(booth());
    obs.inv = vec![row(1590, 1, "Dusty key", -1, false)];
    let token = begin();
    let proj = site(json!({}));
    let step = reach_open(token, &proj, obs);
    assert_ne!(kind(&step), "withdraw", "{step}");
    assert_ne!(kind(&step), "log", "{step}");
    assert!(!step.to_string().contains("Velrak"), "{step}");
    assert!(!step.to_string().contains("fetch"), "{step}");
    assert_not_forbidden(&step);
}

#[test]
fn snapshot_slot_minus_one_is_not_a_real_held_row() {
    reset();
    set_observation(at(booth()));
    let token = begin();
    let proj = site(json!({}));
    let step = call(token, proj.clone(), None);
    assert_eq!(kind(&step), "bank-open", "{step}");
    let ops = Vec::<String>::new();
    let inv = [ItemRowInput {
        name: Some("Dusty key"),
        count: 1,
        id: 1590,
        ops: &ops,
        noted: false,
        cert: -1,
        component_id: -1,
        slot: -1,
    }];
    let mut snap = empty_snapshot(
        1,
        TileInput {
            x: 2946,
            z: 3369,
            level: 0,
        },
    );
    snap.inv = &inv;
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 4;
    let bytes = encode_snapshot(&snap);
    let reader = SnapshotReader::from_bytes(&bytes).unwrap();
    hunt_bank::on_snapshot(&reader);
    let step = call(token, proj, Some(json!({ "opened": true })));
    assert_ne!(kind(&step), "withdraw", "{step}");
    assert!(!step.to_string().contains("Velrak"), "{step}");
    assert_not_forbidden(&step);
}

#[test]
fn failed_bank_withdraw_does_not_count_the_trip() {
    reset();
    let mut obs = at(booth());
    obs.bank = vec![row(1590, 1, "Dusty key", 0, false)];
    let token = begin();
    let proj = site(json!({}));
    let step = reach_open(token, &proj, obs.clone());
    assert_eq!(kind(&step), "withdraw", "{step}");
    assert_eq!(step["name"], "Dusty key");
    assert_eq!(step["action"], "Withdraw-1");
    assert!(step.get("id").is_none(), "name withdraw, not by id: {step}");
    assert_ne!(kind(&step), "withdraw-x");
    assert!(bank_force_bound_reached(token));
    set_observation(open_loaded(obs));
    let failed = call(token, proj, Some(json!({ "queued": true })));
    assert_yield(&failed, false);
    assert_ne!(kind(&failed), "count-bank-trip");
    let remaining = bank_deadline_remaining_ms(token);
    assert!(remaining.is_none() || remaining == Some(0));
    let _ = WITHDRAW_MS;
}

#[test]
fn queued_withdraw_without_a_landing_is_not_proof() {
    reset();
    let mut obs = at(booth());
    obs.bank = vec![row(1590, 1, "Dusty key", 0, false)];
    let token = begin();
    let proj = site(json!({}));
    let step = reach_open(token, &proj, obs.clone());
    assert_eq!(kind(&step), "withdraw", "{step}");
    set_observation(open_loaded(obs));
    let waiting = call(token, proj, Some(json!({ "queued": true })));
    assert_eq!(kind(&waiting), "delay-ticks", "{waiting}");
    assert_ne!(kind(&waiting), "yield");
    assert_ne!(kind(&waiting), "count-bank-trip");
}

#[test]
fn full_pack_shield_deposit_has_no_count_and_is_not_deposit_1() {
    reset();
    let mut obs = at(booth());
    obs.bank = vec![row(11284, 1, "Dragonfire shield", 0, false)];
    obs.bank_side = (0..28)
        .map(|slot| row(385, 1, "Shark", slot, false))
        .collect();
    obs.inv_size = 28;
    let token = begin();
    let proj = site(json!({
        "style": "melee",
        "keyItem": null,
        "foodName": "Shark",
    }));
    let step = reach_open(token, &proj, obs);
    assert_eq!(kind(&step), "deposit", "{step}");
    assert_eq!(step["name"], "Shark");
    assert!(step.get("count").is_none(), "{step}");
    assert!(step.get("action").is_none(), "{step}");
    assert!(!step.to_string().contains("deposit-1"), "{step}");
    assert!(!step.to_string().contains("Deposit-1"), "{step}");
}

#[test]
fn held_key_skips_the_withdraw_and_null_key_does_not_search_1590() {
    reset();
    let mut held = at(booth());
    held.inv = vec![row(1590, 1, "Dusty key", 2, true)];
    let token = begin();
    let proj = site(json!({}));
    let step = reach_open(token, &proj, held);
    assert_ne!(kind(&step), "withdraw", "{step}");
    assert!(!step.to_string().contains("Velrak"), "{step}");

    reset();
    let token = begin();
    let proj = site(json!({
        "key": "heroes-blue",
        "keyItem": null,
        "bank": { "x": 2946, "z": 3369, "level": 0 },
    }));
    let mut obs = at(booth());
    obs.bank = vec![row(1590, 4, "Dusty key", 0, false)];
    let step = reach_open(token, &proj, obs);
    assert_ne!(kind(&step), "withdraw", "{step}");
    assert!(!step.to_string().contains("1590"), "{step}");
    assert!(!step.to_string().contains("Velrak"), "{step}");
}

#[test]
fn fetch_logs_and_continues_and_nameless_yields_false_without_withdraw() {
    reset();
    let token = begin();
    let proj = site(json!({}));
    let step = reach_open(token, &proj, at(booth()));
    assert_eq!(kind(&step), "log", "{step}");
    assert!(
        step["message"].as_str().unwrap_or("").contains("Velrak"),
        "{step}"
    );
    assert_not_forbidden(&step);
    let next = call(token, proj, None);
    assert_ne!(kind(&next), "npc", "{next}");
    assert_ne!(kind(&next), "withdraw", "{next}");

    reset();
    let mut obs = at(booth());
    obs.bank = vec![row(1590, 1, "", 0, false)];
    let token = begin();
    let proj = site(json!({}));
    let step = reach_open(token, &proj, obs);
    assert_yield(&step, false);
    assert_ne!(kind(&step), "withdraw");
    assert_ne!(kind(&step), "count-bank-trip");
}

#[test]
fn jail_key_and_cert_are_not_the_1590_arm() {
    reset();
    let mut obs = at(booth());
    obs.bank = vec![
        row(1591, 1, "Jail key", 0, false),
        row(1592, 1, "Dusty key", 0, false),
    ];
    let token = begin();
    let proj = site(json!({}));
    let step = reach_open(token, &proj, obs);
    assert_ne!(kind(&step), "withdraw", "{step}");
    assert_ne!(kind(&step), "npc", "{step}");
    assert_ne!(kind(&step), "obj", "{step}");
    if kind(&step) == "log" {
        assert!(step["message"].as_str().unwrap_or("").contains("Velrak"));
    }
}

#[test]
fn heal_closes_before_eat_and_a_bite_reopens_for_food() {
    reset();
    let mut obs = at(booth());
    obs.hp_effective = 10;
    obs.bank_side = vec![row(385, 1, "Shark", 0, false)];
    obs.inv = vec![row(385, 1, "Shark", 1, true)];
    let token = begin();
    let proj = site(json!({
        "keyItem": null,
        "withdrawFood": true,
        "foodWant": 1,
        "foodName": "Shark",
    }));
    let step = reach_open(token, &proj, obs.clone());
    assert_eq!(kind(&step), "close", "eat must not run while open: {step}");
    assert_ne!(kind(&step), "held");
    let mut closed = obs.clone();
    closed.bank_open = false;
    closed.bank_loaded = false;
    closed.bank.clear();
    closed.bank_side.clear();
    closed.inv = vec![row(385, 1, "Shark", 1, true)];
    set_observation(closed.clone());
    let eat = call(token, proj.clone(), Some(json!({ "queued": true })));
    assert_eq!(kind(&eat), "held", "{eat}");
    assert_eq!(eat["action"], "Eat");
    assert_ne!(kind(&eat), "eat");
    closed.inv = vec![row(385, 0, "Shark", 1, true)];
    closed.hp_effective = 40;
    set_observation(closed);
    let topup = call(token, proj, Some(json!({ "queued": true })));
    assert_eq!(kind(&topup), "bank-open", "post-heal food top-up: {topup}");
    assert_ne!(kind(&topup), "count-bank-trip");
    assert_ne!(kind(&topup), "yield");
}

#[test]
fn wear_closes_only_when_inventory_is_empty() {
    reset();
    let mut hidden = at(booth());
    hidden.bank_side = vec![wearable(1333, "Rune scimitar", 0)];
    hidden.inv = vec![row(0, 0, "", -1, false)];
    let token = begin();
    let proj = site(json!({
        "keyItem": null,
        "style": "range",
        "weapon": "Rune scimitar",
    }));
    let step = reach_open(token, &proj, hidden);
    assert_eq!(kind(&step), "close", "do not wear a deposit row: {step}");
    assert_ne!(kind(&step), "wear");

    reset();
    let mut visible = at(booth());
    visible.inv = vec![wearable(1333, "Rune scimitar", 3)];
    let token = begin();
    let step = reach_open(token, &proj, visible);
    assert_eq!(kind(&step), "wear", "{step}");
    assert_ne!(kind(&step), "close");
    assert_eq!(step["name"], "Rune scimitar");
}

#[test]
fn changed_generation_is_not_a_deposit_and_close_queued_is_not_done() {
    reset();
    let mut obs = at(booth());
    obs.bank_side = vec![row(526, 2, "Bones", 1, false)];
    let token = begin();
    let proj = site(json!({ "keyItem": null }));
    let step = reach_open(token, &proj, obs.clone());
    assert_eq!(kind(&step), "deposit", "{step}");
    assert_eq!(step["name"], "Bones");
    assert!(step.get("count").is_none(), "{step}");
    let mut stale = open_loaded(obs.clone());
    stale.bank_generation = 99;
    stale.bank_op_result = true;
    stale.bank_op_result_seq = 8;
    stale.bank_side.clear();
    set_observation(stale);
    let failed = call(token, proj.clone(), Some(json!({ "queued": true })));
    assert_yield(&failed, false);
    assert_ne!(kind(&failed), "count-bank-trip");

    reset();
    let token = begin();
    let step = reach_open(token, &proj, at(booth()));
    assert_eq!(kind(&step), "close", "{step}");
    let mut still = open_loaded(at(booth()));
    still.bank_open = true;
    set_observation(still);
    let waiting = call(token, proj.clone(), Some(json!({ "queued": true })));
    assert_eq!(kind(&waiting), "delay-ticks", "{waiting}");
    assert_ne!(kind(&waiting), "yield");
    assert_ne!(kind(&waiting), "count-bank-trip");
    let mut shut = at(booth());
    shut.bank_open = false;
    shut.bank.clear();
    set_observation(shut);
    let count = call(token, proj.clone(), None);
    assert_eq!(kind(&count), "count-bank-trip", "{count}");
    let done = call(token, proj, None);
    assert_yield(&done, true);
}

#[test]
fn junk_remaining_after_close_does_not_count() {
    reset();
    let mut obs = at(booth());
    obs.bank_side = vec![row(526, 1, "Bones", 1, false)];
    let token = begin();
    let proj = site(json!({ "keyItem": null }));
    let step = reach_open(token, &proj, obs.clone());
    assert_eq!(kind(&step), "deposit", "{step}");
    let mut proved = open_loaded(obs);
    proved.bank_side.clear();
    proved.bank_op_result = true;
    proved.bank_op_result_seq = 3;
    set_observation(proved);
    let close = call(token, proj.clone(), Some(json!({ "queued": true })));
    assert_eq!(kind(&close), "close", "{close}");
    let mut shut = at(booth());
    shut.inv = vec![row(526, 1, "Bones", 2, true)];
    set_observation(shut);
    let failed = call(token, proj, Some(json!({ "queued": true })));
    assert_yield(&failed, false);
    assert_ne!(kind(&failed), "count-bank-trip");
}

#[test]
fn kbd_route_does_not_emit_loc_and_hold_yields_false() {
    reset();
    let token = begin();
    let proj = site(json!({
        "keyItem": null,
        "route": [{ "locId": 1765 }, { "id": 1816 }],
    }));
    let step = reach_open(token, &proj, at(booth()));
    assert_ne!(kind(&step), "loc", "{step}");
    assert_not_forbidden(&step);
    let mut held = at(booth());
    held.hold = true;
    set_observation(held);
    let stopped = call(token, proj, Some(json!({ "queued": true })));
    assert_yield(&stopped, false);
    assert_ne!(kind(&stopped), "count-bank-trip");
}

#[test]
fn begin_allocates_a_new_token_and_pause_emits_wait() {
    reset();
    set_observation(at(tile(50, 50, 0)));
    let first = begin();
    let second = begin();
    assert_ne!(first, second);
    assert!(bank_token_alive(first));
    assert!(bank_token_alive(second));
    let proj = site(json!({}));
    assert_eq!(kind(&call(first, proj.clone(), None)), "leave");
    assert!(bank_token_alive(first));
    hunt_bank::on_pause();
    assert_eq!(kind(&call(second, proj.clone(), None)), "wait");
    hunt_bank::on_resume();
    assert_eq!(kind(&call(second, proj.clone(), None)), "leave");
    let ended = hunt_bank::dispatch(&json!({ "op": "end", "token": first }));
    assert_eq!(kind(&ended), "ok", "{ended}");
    assert!(!bank_token_alive(first), "end drops that row: {ended}");
    assert!(bank_token_alive(second), "end is not a map clear: {ended}");
    let again = hunt_bank::dispatch(&json!({ "op": "end", "token": first }));
    assert_eq!(kind(&again), "ok", "an unknown token is a no-op: {again}");
    hunt_bank::on_reset();
    assert!(!bank_token_alive(first));
    assert!(!bank_token_alive(second));
    let aborted = call(first, proj, None);
    assert_eq!(kind(&aborted), "aborted");
    let _ = CLOSE_MS;
}

#[test]
fn source_owns_its_map_and_does_not_call_the_other_machines() {
    let src = include_str!("../src/hunt_bank.rs");
    assert!(src.contains("BANK_RUNTIMES"));
    assert!(src.contains("in_area_body(here, 1,"));
    assert!(!src.contains("hunt_leave::dispatch"));
    assert!(!src.contains("hunt_cell::dispatch"));
    assert!(!src.contains("hunt_key::dispatch"));
    assert!(!src.contains("hunt_lair::dispatch"));
    assert!(!src.contains("hunt_fight::dispatch"));
    assert!(!src.contains("deposit-1"));
    assert!(!src.contains("withdraw-by-id"));
    assert!(!src.contains("walk-nearest-bank"));
    assert!(!src.contains("FIGHT_MS"));
    assert!(!src.contains("120000"));
    assert!(!src.contains("120_000"));
    assert!(!src.contains("SelectedGameData"));
    assert!(!src.contains("gap_sw"));
    assert!(!src.contains("bodyOrigin"));
    assert!(!src.contains("size>>1"));
    assert_eq!(WITHDRAW_MS, 2_500);
    assert_eq!(CLOSE_MS, 3_000);
    assert_eq!(OPEN_MS, 5_000);
    assert_eq!(WALK_LEG_MS, 300_000);
    let lib = include_str!("../src/lib.rs");
    assert_eq!(lib.matches("pub mod hunt_bank").count(), 1);
}

#[test]
fn shim_and_bindings_keep_the_yield_shape_and_walk_flags() {
    let bindings = include_str!("../src/load/bindings.rs");
    let bank = bindings.split("api.bankNext").nth(1).expect("bankNext");
    let bank = bank.split("function recordSettlement").next().unwrap();
    assert!(bank.contains("kind: 'yield', value:"));
    assert!(bank.contains("ok: false, error:"));
    assert!(bank.contains("kind: 'aborted'"));
    assert!(bank.contains("status: 'aborted'"));
    assert!(!bank.contains("ok: true, status: 'aborted'"));
    assert!(!bindings.contains("api.bankValidate"));
    let reg = bindings.split("__rs2b0t_bank\"").nth(1).unwrap();
    let reg = reg.split("__rs2b0t_production").next().unwrap();
    assert!(!reg.contains("SelectedGameData"));
    assert!(!reg.contains("selected_"));

    let dts = include_str!("../src/host_js.rs");
    let step = dts.split("export type BankStep").nth(1).expect("BankStep");
    assert!(step.contains("kind: 'yield'; value: boolean"));
    assert!(step.contains("ok: false; error: string; kind: 'aborted'"));
    assert!(!dts.contains("bankValidate"));

    let js = include_str!("../src/shim/hunting_combat.js");
    let routine = js
        .split("export async function bankRoutine")
        .nth(1)
        .expect("bankRoutine");
    assert!(routine.contains("driveSiteBankOpen"));
    assert!(routine.contains("leaveLair"));
    assert!(routine.contains("opts.leave"));
    assert!(routine.contains("{ left:"));
    assert!(routine.contains("Use-quickly") || js.contains("booth_action: 'Use-quickly'"));
    let driver = js.split("async function driveSiteBankOpen").nth(1).unwrap();
    let driver = driver
        .split("export async function bankRoutine")
        .next()
        .unwrap();
    assert!(driver.contains("open-nearest"));
    assert!(driver.contains("Bank booth"));
    assert!(driver.contains("Use-quickly"));
    assert!(!driver.contains("walk-nearest-bank"));
    assert!(routine.contains("countBankTrip"));
    assert!(!routine.contains("keyStatus"));
    assert!(!routine.contains("Inventory"));
    assert!(!routine.contains("1590"));
    assert!(!routine.contains("deposit-1"));
    assert!(!routine.contains("withdraw-by-id"));
    assert!(!routine.contains("Bank.deposit"));
    assert!(!routine.contains("Bank.close"));
    assert!(!routine.contains("Bank.withdrawById"));
    assert!(!routine.contains("walk-nearest-bank"));
    let walk = routine.split("case 'walk-near'").nth(1).unwrap();
    let walk = walk.split("break;").next().unwrap();
    assert!(walk.contains("beginBankWalk(step, radius)"));
    assert!(walk.contains("allow_teleports: false"));
    assert!(walk.contains("allow_wilderness: false"));
    assert!(walk.contains("allow_bank_fetch: false"));
    assert!(!walk.contains("allow_wilderness: true"));
    assert!(!walk.contains("allow_bank_fetch: true"));
    assert!(!walk.contains("radius: 0"));
    let begin = js.split("function beginBankWalk").nth(1).unwrap();
    let begin = begin
        .split("async function driveSiteBankOpen")
        .next()
        .unwrap();
    assert!(begin.contains("allow_teleports: false"));
    assert!(begin.contains("allow_wilderness: false"));
    assert!(begin.contains("allow_bank_fetch: false"));
    assert!(begin.contains("radius,"));
    assert_eq!(
        routine.matches("bankCall({ op: 'end', token })").count(),
        routine.matches("return ").count() - 1,
        "every return after the begin ends the Rust row: {routine}"
    );

    let iso = include_str!("../src/load/isolate.rs");
    for hook in [
        "on_snapshot",
        "on_pause",
        "on_hold",
        "on_resume",
        "on_reset",
    ] {
        assert!(
            iso.contains(&format!("hunt_bank::{hook}")),
            "missing {hook}"
        );
        assert!(iso.contains(&format!("hunt_cell::{hook}")));
    }
}

#[test]
fn v2_bank_next_keeps_abort_distinct_from_yield_false() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const began = api.bankBegin();
  globalThis.__begin = began;
  const token = began.value.token;
  const bad = api.bankNext({
    token: token + 1,
    key: 'heroes-blue',
    keyItem: null,
  });
  globalThis.__bad = bad;
  const leave = api.bankNext({
    token,
    key: 'taverley-blue',
    keyItem: null,
    boxes: [{ minX: 0, maxX: 10, minZ: 0, maxZ: 10, level: 0 }],
    bank: { x: 80, z: 80, level: 0 },
  });
  globalThis.__leave = leave;
  const done = api.bankNext({
    token,
    reply: { left: false },
    key: 'taverley-blue',
    keyItem: null,
    boxes: [{ minX: 0, maxX: 10, minZ: 0, maxZ: 10, level: 0 }],
    bank: { x: 80, z: 80, level: 0 },
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
    assert!(bad.get("value").is_none(), "{bad:?}");
    let leave = iso.probe("globalThis.__leave").unwrap();
    assert_eq!(leave["ok"], true, "{leave:?}");
    assert_eq!(leave["status"], "continue");
    assert_eq!(leave["kind"], "leave", "{leave:?}");
    let done = iso.probe("globalThis.__done").unwrap();
    assert_eq!(done["ok"], true, "{done:?}");
    assert_eq!(done["status"], "done");
    assert_eq!(done["kind"], "yield");
    assert_eq!(done["value"], false);
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
