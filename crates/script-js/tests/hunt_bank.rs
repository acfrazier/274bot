//! Offline Bank isolate policy tests. No LIVE.
//!
//! These lock the architecture corrections: slot -1 is not fetch, a failed
//! bank withdraw does not count the trip, the yield shape, no deposit-1, and
//! walk-near radius 3 with the three allow flags false. They do not mirror
//! phases.

use script_js::hunt_bank::{
    self, bank_deadline_remaining_ms, bank_force_bound_reached, bank_token_alive, set_observation,
    BankObservation, BankRow, APPROACH_RADIUS, CLOSE_MS, OPEN_MS, WALK_LEG_MS, WITHDRAW_MS,
};
use script_js::hunt_fight::Tile;
use script_js::wire::{encode_snapshot, ItemRowInput, Snapshot, SnapshotInput, TileInput};
use script_js::LoadShape;
use serde_json::{json, Value};

#[path = "common/bank_slot.rs"]
mod bank_slot;

fn bank_slot(source: &str) -> bank_slot::BankSlot {
    bank_slot::BankSlot::new(
        source,
        LoadShape::NativeTick,
        None,
        std::sync::Arc::new(api::named_banks::NamedBankFacts::empty()),
    )
}

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
        noted: false,
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
    let reader = Snapshot::from_bytes(&bytes).unwrap();
    script_js::observed::apply(&reader);
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
    let restock = call(token, proj.clone(), None);
    assert_eq!(kind(&restock), "status", "{restock}");
    assert!(
        restock["message"]
            .as_str()
            .unwrap_or("")
            .starts_with("restocked, heading back"),
        "{restock}"
    );
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
fn v2_bank_run_calls_the_leave_hook_and_stops_when_it_fails() {
    let src = r#"
export const apiVersion = 2;
let started = false;
export async function tick(api) {
  if (started) return;
  started = true;
  globalThis.__leaves = 0;
  globalThis.__done = await api.bankRun({
    key: 'taverley-blue',
    keyItem: null,
    boxes: [{ minX: 0, maxX: 10, minZ: 0, maxZ: 10, level: 0 }],
    bank: { x: 80, z: 80, level: 0 },
  }, {}, { leave: async () => { globalThis.__leaves += 1; return false; } });
}
"#;
    let mut slot = bank_slot(src);
    for tick in 1..=2 {
        slot.tick_from_input(
            tick,
            &empty_snapshot(
                tick,
                TileInput {
                    x: 1,
                    z: 1,
                    level: 0,
                },
            ),
            script_js::wire::NativeFactsInput::default(),
        );
    }
    // Inside the lair, the caller's own leave runs; a failed leave ends
    // the trip false without walking to the bank.
    assert_eq!(slot.probe("globalThis.__leaves").unwrap(), json!(1));
    assert_eq!(
        slot.probe("globalThis.__done").unwrap(),
        json!({ "kind": "done", "value": false })
    );
    assert!(
        !slot
            .take_actions()
            .iter()
            .any(|action| matches!(action.effect, script::native::HostEffect::Walk { .. })),
        "no bank walk after a failed leave"
    );
    let (_, legacy, _) = slot.slot.drain_host_interacts();
    assert!(!legacy
        .iter()
        .any(|row| matches!(row.req, script::shim::InteractReq::WalkNear { .. })));
    slot.stop();
}

#[test]
fn v2_bank_run_uses_the_native_bank_open_child_and_bounds_a_rejected_dispatch() {
    use api::quest_progress::EvidenceStamp;
    use script::native::{HostEffect, InteractionReceipt};
    use script_js::wire::{NativeFactsInput, ReachViewInput, SceneEntityInput};

    let source = r#"
export const apiVersion = 2;
let started = false;
export async function tick(api) {
    if (started) return;
    started = true;
    globalThis.__done = await api.bankRun({
        key: 'heroes-blue',
        keyItem: null,
        boxes: [],
        bank: { x: 80, z: 80, level: 0 },
        style: 'range',
    }, {}, {});
}
"#;
    let actions = vec!["Use-quickly".to_owned()];
    let booths = [SceneEntityInput {
        index: 2213,
        id: 2213,
        name: Some("Bank booth"),
        x: 80,
        z: 81,
        level: 0,
        distance: 1,
        health: -1,
        max_health: -1,
        in_combat: false,
        animating: false,
        actions: &actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 0,
        target_kind: 0,
        target_index: -1,
        size: 0,
        nx: 0,
        nz: 0,
        shape: 10,
        angle: 0,
    }];
    let booth_tiles = [TileInput {
        x: 80,
        z: 81,
        level: 0,
    }];
    let mut snapshot = empty_snapshot(
        1,
        TileInput {
            x: 80,
            z: 80,
            level: 0,
        },
    );
    snapshot.booths = &booth_tiles;
    snapshot.locs = &booths;
    snapshot.reach = ReachViewInput {
        available: true,
        base_x: 76,
        base_z: 76,
        level: 0,
        width: 8,
        height: 8,
        walkable: &[u32::MAX; 2],
        reachable: &[u32::MAX; 2],
        reachable_adj: &[u32::MAX; 2],
        exact_rank: &[0; 64],
        adjacent_rank: &[0; 64],
        step: &[u8::MAX; 64],
        canlight: &[0; 2],
        stamp: 1,
    };
    let mut slot = bank_slot(source);
    let mut sent = None;
    for tick in 1..=8 {
        snapshot.tick = tick;
        slot.tick_from_input(tick, &snapshot, NativeFactsInput::default());
        let mut actions = slot.take_actions();
        if !actions.is_empty() {
            assert_eq!(actions.len(), 1, "one native booth attempt");
            sent = Some((actions.pop().unwrap(), tick));
            break;
        }
    }
    let (action, tick) = sent.expect("Hunt bank-open must reach the native seat");
    assert!(matches!(
        action.effect,
        HostEffect::Interaction(script::shim::InteractReq::OpenBooth { id: 2213, .. })
    ));
    assert_eq!(
        slot.probe("typeof globalThis.__done").unwrap(),
        json!("undefined")
    );
    let authority = action.authority();
    slot.slot.complete_native_interaction(
        &authority,
        InteractionReceipt {
            request_id: action.request_id.get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            accepted: false,
            chat_since: 0,
        },
    );
    // The frozen one-shot booth path observes freshness for five seconds even
    // when the host rejects its click. Exercise that real active-time bound.
    std::thread::sleep(std::time::Duration::from_millis(5_100));
    for next in tick + 1..=tick + 8 {
        snapshot.tick = next;
        slot.tick_from_input(next, &snapshot, NativeFactsInput::default());
        assert!(
            slot.take_actions().is_empty(),
            "a failed opener is not retried"
        );
        if slot
            .probe("typeof globalThis.__done !== 'undefined'")
            .unwrap()
            == json!(true)
        {
            break;
        }
    }
    assert_eq!(
        slot.probe("globalThis.__done").unwrap(),
        json!({ "kind": "done", "value": false })
    );
    slot.stop();
}

fn empty_snapshot<'a>(tick: u64, here: TileInput) -> SnapshotInput<'a> {
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
        reach: script_js::wire::ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
        user_move_intent_seq: 0,
        walk_outcome_cancel_reason: Default::default(),
    }
}

fn noted_row(id: i32, count: i32, name: &str) -> BankRow {
    let mut item = row(id, count, name, 0, false);
    item.noted = true;
    item
}

/// The potion-restock shape (live `jive_dragons_potion_restock`): the pack
/// holds noted food and a part-used flask the card's keep list does not
/// keep. The deposit stage must deposit both — noted items deposit even
/// though the food name is kept (frozen `depositLoot` matches notes by
/// id), and the partial dose deposits because only the full flask form is
/// in `keepExtra` — after which the flask stage draws the full form.
#[test]
fn noted_food_and_partial_flask_deposit_then_full_flask_draws() {
    reset();
    let mut obs = at(booth());
    obs.bank_side = vec![
        noted_row(380, 20, "Lobster"),
        row(149, 1, "Super attack(1)", 1, false),
    ];
    obs.bank = vec![
        row(379, 40, "Lobster", 0, false),
        row(145, 2, "Super attack(3)", 1, false),
    ];
    obs.equipment = vec![row(1540, 1, "Dragonfire shield", 0, true)];
    let token = begin();
    let proj = site(json!({
        "keyItem": null,
        "foodName": "Lobster",
        "style": "melee",
        "foodWant": 0,
        "keepExtra": ["Super attack(3)"],
        "flasks": [{
            "flask": "Super attack(3)",
            "doses": ["Super attack(4)", "Super attack(3)", "Super attack(2)", "Super attack(1)"],
            "want": 1,
        }],
    }));
    let step = reach_open(token, &proj, obs.clone());
    assert_eq!(kind(&step), "deposit", "noted food deposits first: {step}");
    assert_eq!(step["name"], "Lobster", "{step}");

    let mut proved = open_loaded(obs);
    proved.bank_side = vec![row(149, 1, "Super attack(1)", 1, false)];
    proved.bank_op_result = true;
    proved.bank_op_result_seq = 3;
    set_observation(proved.clone());
    let step = call(token, proj.clone(), Some(json!({ "queued": true })));
    assert_eq!(
        kind(&step),
        "deposit",
        "partial flask deposits next: {step}"
    );
    assert_eq!(step["name"], "Super attack(1)", "{step}");

    proved.bank_side.clear();
    proved.bank_op_result_seq = 5;
    set_observation(proved);
    let mut kinds = Vec::new();
    let mut drew_full = false;
    for _ in 0..16 {
        let step = call(token, proj.clone(), Some(json!({ "queued": true })));
        kinds.push(format!(
            "{}:{}",
            kind(&step),
            step["name"].as_str().unwrap_or("")
        ));
        if kind(&step) == "withdraw" && step["name"] == "Super attack(3)" {
            drew_full = true;
            break;
        }
        if kind(&step) == "yield" {
            break;
        }
    }
    assert!(
        drew_full,
        "flask stage draws the full form after the partial deposits: {kinds:?}"
    );
}

const LOBSTER: i32 = 379;
const NOTED_LOBSTER: i32 = 380;
const BONES: i32 = 526;
const SUPER_ATTACK_3: i32 = 145;
const NOTED_SUPER_ATTACK_3: i32 = 146;

/// A bank the kernel's effects act on, so one trip runs from the booth to
/// its yield. Deposit selects the side row the way the host does
/// (`host-play` `dispatch_observed_bank_op`): the row with the effect's `id`
/// and name when the effect carries an id, else the first side row with the
/// name; Deposit-All then moves every copy of that row's id. The pack is one
/// row per item id, which keeps the counts the kernel reads.
struct TripSim {
    pack: Vec<BankRow>,
    bank: Vec<BankRow>,
    open: bool,
    seq: u64,
    x_seq: u64,
    deposited: Vec<i32>,
    trace: Vec<String>,
}

impl TripSim {
    fn new(pack: Vec<BankRow>, bank: Vec<BankRow>) -> Self {
        Self {
            pack,
            bank,
            open: false,
            seq: 1,
            x_seq: 0,
            deposited: Vec::new(),
            trace: Vec::new(),
        }
    }

    fn observation(&self) -> BankObservation {
        let mut obs = at(booth());
        let slotted = |has_slot: bool| -> Vec<BankRow> {
            self.pack
                .iter()
                .enumerate()
                .map(|(slot, row)| BankRow {
                    slot: slot as i32,
                    has_slot,
                    ..row.clone()
                })
                .collect()
        };
        obs.inv = slotted(true);
        if self.open {
            obs = open_loaded(obs);
            obs.bank = self.bank.clone();
            obs.bank_side = slotted(false);
        }
        obs.bank_op_result = self.seq > 1;
        obs.bank_op_result_seq = self.seq;
        obs.withdraw_x_result = self.x_seq > 0;
        obs.withdraw_x_result_seq = self.x_seq;
        obs
    }

    fn held(&self, id: i32) -> i32 {
        self.pack
            .iter()
            .filter(|row| row.id == id)
            .map(|row| row.count)
            .sum()
    }

    fn take_from_bank(&mut self, name: &str, id: Option<i32>, count: Option<i32>) {
        let Some(row) = self.bank.iter_mut().find(|row| {
            row.count > 0 && id.is_none_or(|id| row.id == id) && row.name.eq_ignore_ascii_case(name)
        }) else {
            return;
        };
        let moved = count.map_or(row.count, |count| count.min(row.count));
        row.count -= moved;
        let taken = BankRow {
            count: moved,
            ..row.clone()
        };
        match self.pack.iter_mut().find(|held| held.id == taken.id) {
            Some(held) => held.count += moved,
            None => self.pack.push(taken),
        }
    }

    /// Apply one effect; the result is the reply the next call carries.
    fn apply(&mut self, step: &Value) -> Option<Value> {
        let name = step["name"].as_str().unwrap_or("");
        let queued = Some(json!({ "queued": true }));
        match kind(step) {
            "bank-open" => {
                self.open = true;
                Some(json!({ "opened": true }))
            }
            "deposit" => {
                let id = step["id"].as_i64().map(|id| id as i32);
                let picked = self
                    .pack
                    .iter()
                    .find(|row| {
                        id.is_none_or(|id| row.id == id) && row.name.eq_ignore_ascii_case(name)
                    })
                    .cloned();
                if let Some(row) = picked {
                    let moved = self.held(row.id);
                    self.pack.retain(|held| held.id != row.id);
                    match self.bank.iter_mut().find(|stock| stock.id == row.id) {
                        Some(stock) => stock.count += moved,
                        None => self.bank.push(BankRow {
                            count: moved,
                            ..row.clone()
                        }),
                    }
                    self.deposited.push(row.id);
                }
                self.seq += 1;
                queued
            }
            "withdraw" => {
                let action = step["action"].as_str().unwrap_or("");
                let count = match action.to_ascii_lowercase().replace(' ', "-").as_str() {
                    "withdraw-1" => Some(1),
                    "withdraw-5" => Some(5),
                    "withdraw-10" => Some(10),
                    "withdraw-all" => None,
                    other => panic!("unexpected withdraw action {other}: {step}"),
                };
                self.take_from_bank(name, None, count);
                self.seq += 1;
                queued
            }
            "withdraw-x" => {
                let id = step["bank_item_id"].as_i64().map(|id| id as i32);
                let count = step["count"].as_i64().map(|count| count as i32);
                self.take_from_bank(name, id, count);
                self.x_seq += 1;
                queued
            }
            "close" => {
                self.open = false;
                queued
            }
            "delay-ticks" | "wait" | "log" | "status" | "count-bank-trip" => None,
            other => panic!("the trip simulation does not model {other}: {step}"),
        }
    }
}

/// Run one bank trip from the booth until the kernel yields; `None` when it
/// never does within the step cap.
fn run_trip(proj: &Value, sim: &mut TripSim) -> Option<bool> {
    reset();
    let token = begin();
    let mut reply = None;
    for _ in 0..400 {
        set_observation(sim.observation());
        let step = call(token, proj.clone(), reply.take());
        assert_not_forbidden(&step);
        sim.trace.push(format!(
            "{}:{}{}",
            kind(&step),
            step["name"].as_str().unwrap_or(""),
            step["id"]
                .as_i64()
                .map_or(String::new(), |id| format!("#{id}"))
        ));
        match kind(&step) {
            "yield" => return step["value"].as_bool(),
            "aborted" => panic!("trip aborted: {step}; {:?}", sim.trace),
            _ => reply = sim.apply(&step),
        }
    }
    None
}

fn swapped<T: Clone>(rows: &[T; 2]) -> [T; 2] {
    [rows[1].clone(), rows[0].clone()]
}

fn lobster_trip(food_want: i32) -> Value {
    site(json!({
        "keyItem": null,
        "foodName": "Lobster",
        "withdrawFood": true,
        "foodWant": food_want,
        "keepExtra": ["Super attack(3)"],
    }))
}

/// R1: with edible and noted Lobsters on the pack in either order, the trip
/// deposits exactly the noted row and the Bones, keeps the edible food and
/// the explicitly kept flask, and completes.
#[test]
fn mixed_lobster_rows_deposit_only_the_notes_in_either_order() {
    let mixed = [
        row(LOBSTER, 5, "Lobster", 0, false),
        noted_row(NOTED_LOBSTER, 20, "Lobster"),
    ];
    for food in [mixed.clone(), swapped(&mixed)] {
        let mut pack = food.to_vec();
        pack.push(row(SUPER_ATTACK_3, 1, "Super attack(3)", 0, false));
        pack.push(row(BONES, 3, "Bones", 0, false));
        let mut sim = TripSim::new(pack, vec![row(LOBSTER, 40, "Lobster", 0, false)]);
        let done = run_trip(&lobster_trip(8), &mut sim);
        let order = &food[0].noted;
        assert_eq!(done, Some(true), "noted first: {order}; {:?}", sim.trace);
        let mut deposited = sim.deposited.clone();
        deposited.sort_unstable();
        assert_eq!(
            deposited,
            vec![NOTED_LOBSTER, BONES],
            "only the notes and the loot deposit: {:?}",
            sim.trace
        );
        assert_eq!(sim.held(NOTED_LOBSTER), 0, "{:?}", sim.trace);
        assert!(sim.held(LOBSTER) >= 8, "{:?}", sim.trace);
        assert_eq!(sim.held(SUPER_ATTACK_3), 1, "the kept flask stays");
        assert!(
            sim.trace.iter().any(|step| step == "count-bank-trip:"),
            "{:?}",
            sim.trace
        );
    }
}

/// R1, potion form: an explicitly kept flask name with noted and unnoted
/// rows in either order deposits the notes only and keeps the flask.
#[test]
fn kept_flask_name_with_noted_and_unnoted_rows_deposits_only_the_notes() {
    let rows = [
        row(SUPER_ATTACK_3, 1, "Super attack(3)", 0, false),
        noted_row(NOTED_SUPER_ATTACK_3, 5, "Super attack(3)"),
    ];
    let proj = site(json!({
        "keyItem": null,
        "foodName": "Lobster",
        "keepExtra": ["Super attack(3)"],
        "flasks": [{
            "flask": "Super attack(3)",
            "doses": ["Super attack(4)", "Super attack(3)", "Super attack(2)", "Super attack(1)"],
            "want": 1,
        }],
    }));
    for pack in [rows.clone(), swapped(&rows)] {
        let mut sim = TripSim::new(
            pack.to_vec(),
            vec![row(SUPER_ATTACK_3, 3, "Super attack(3)", 0, false)],
        );
        let done = run_trip(&proj, &mut sim);
        assert_eq!(done, Some(true), "{:?}", sim.trace);
        assert_eq!(sim.deposited, vec![NOTED_SUPER_ATTACK_3], "{:?}", sim.trace);
        assert_eq!(sim.held(SUPER_ATTACK_3), 1, "{:?}", sim.trace);
        assert_eq!(sim.held(NOTED_SUPER_ATTACK_3), 0, "{:?}", sim.trace);
    }
}

/// R1 proof: a same-name row leaving the pack is not the selected deposit.
/// The kernel names the noted row; when the edible row leaves instead, the
/// op stays pending instead of advancing to the next stage.
#[test]
fn another_same_name_row_leaving_does_not_prove_the_selected_deposit() {
    reset();
    let mut obs = at(booth());
    obs.bank_side = vec![
        row(LOBSTER, 5, "Lobster", 0, false),
        noted_row(NOTED_LOBSTER, 20, "Lobster"),
    ];
    obs.bank = vec![row(LOBSTER, 40, "Lobster", 0, false)];
    let token = begin();
    let proj = lobster_trip(8);
    let step = reach_open(token, &proj, obs.clone());
    assert_eq!(kind(&step), "deposit", "{step}");
    assert_eq!(step["name"], "Lobster", "{step}");
    assert_eq!(
        step["id"], NOTED_LOBSTER,
        "the effect names the noted row: {step}"
    );

    let mut wrong = open_loaded(obs);
    wrong.bank_side = vec![noted_row(NOTED_LOBSTER, 20, "Lobster")];
    wrong.bank_op_result = true;
    wrong.bank_op_result_seq = 3;
    set_observation(wrong.clone());
    let step = call(token, proj.clone(), Some(json!({ "queued": true })));
    assert_eq!(
        kind(&step),
        "delay-ticks",
        "the edible row leaving is not proof: {step}"
    );

    wrong.bank_side.clear();
    wrong.bank_op_result_seq = 4;
    set_observation(wrong);
    let step = call(token, proj, None);
    assert_ne!(
        kind(&step),
        "delay-ticks",
        "the notes leaving is proof: {step}"
    );
    assert_ne!(kind(&step), "yield", "{step}");
}

/// Food withdrawal after a same-name note deposit: the edible Lobsters stay,
/// and the food stage tops them up to `foodWant`.
#[test]
fn food_withdrawal_tops_up_edible_food_kept_beside_deposited_notes() {
    let mut sim = TripSim::new(
        vec![
            row(LOBSTER, 2, "Lobster", 0, false),
            noted_row(NOTED_LOBSTER, 20, "Lobster"),
        ],
        vec![row(LOBSTER, 40, "Lobster", 0, false)],
    );
    let done = run_trip(&lobster_trip(8), &mut sim);
    assert_eq!(done, Some(true), "{:?}", sim.trace);
    assert_eq!(sim.deposited, vec![NOTED_LOBSTER], "{:?}", sim.trace);
    assert!(
        sim.trace
            .iter()
            .any(|step| step.starts_with("withdraw:Lobster")),
        "{:?}",
        sim.trace
    );
    assert!(sim.held(LOBSTER) >= 8, "{:?}", sim.trace);
}

/// Noted food is not food: with only notes on the pack the food stage
/// still withdraws edible Lobsters to `foodWant`.
#[test]
fn noted_only_food_does_not_count_toward_food_want() {
    let mut sim = TripSim::new(
        vec![noted_row(NOTED_LOBSTER, 20, "Lobster")],
        vec![row(LOBSTER, 40, "Lobster", 0, false)],
    );
    let done = run_trip(&lobster_trip(8), &mut sim);
    assert_eq!(done, Some(true), "{:?}", sim.trace);
    assert_eq!(sim.deposited, vec![NOTED_LOBSTER], "{:?}", sim.trace);
    assert!(sim.held(LOBSTER) >= 8, "{:?}", sim.trace);
}

/// Final readiness: edible food of a deposited noted name does not block the
/// trip, and a noted row still held after the close does.
#[test]
fn final_readiness_accepts_kept_food_and_rejects_a_held_note() {
    let mut sim = TripSim::new(
        vec![
            row(LOBSTER, 8, "Lobster", 0, false),
            noted_row(NOTED_LOBSTER, 20, "Lobster"),
        ],
        vec![row(LOBSTER, 40, "Lobster", 0, false)],
    );
    let done = run_trip(&lobster_trip(8), &mut sim);
    assert_eq!(done, Some(true), "{:?}", sim.trace);
    assert_eq!(sim.held(LOBSTER), 8, "{:?}", sim.trace);

    reset();
    let mut obs = at(booth());
    obs.bank_side = vec![
        row(LOBSTER, 8, "Lobster", 0, false),
        noted_row(NOTED_LOBSTER, 20, "Lobster"),
    ];
    let edible = row(LOBSTER, 8, "Lobster", 1, true);
    let held_note = BankRow {
        slot: 2,
        has_slot: true,
        ..noted_row(NOTED_LOBSTER, 20, "Lobster")
    };
    obs.inv = vec![edible.clone(), held_note.clone()];
    let token = begin();
    let proj = lobster_trip(8);
    let step = reach_open(token, &proj, obs.clone());
    assert_eq!(step["id"], NOTED_LOBSTER, "{step}");
    let mut proved = open_loaded(obs);
    proved.bank_side = vec![row(LOBSTER, 8, "Lobster", 0, false)];
    proved.inv = vec![edible.clone()];
    proved.bank_op_result = true;
    proved.bank_op_result_seq = 3;
    set_observation(proved);
    let close = call(token, proj.clone(), Some(json!({ "queued": true })));
    assert_eq!(kind(&close), "close", "{close}");
    let mut shut = at(booth());
    shut.inv = vec![edible, held_note];
    set_observation(shut);
    let failed = call(token, proj, Some(json!({ "queued": true })));
    assert_yield(&failed, false);
    assert_ne!(kind(&failed), "count-bank-trip");
}

/// The keep-all-dose policy (frozen `JiveDragons.ts:409-410` outside melee):
/// every dose form is in `keepExtra`, so the unnoted partials stay and count
/// toward the flask target; a noted partial of the same name still deposits.
#[test]
fn keep_all_dose_policy_keeps_partials_and_deposits_their_notes() {
    const STRENGTH_2: i32 = 159;
    const NOTED_STRENGTH_2: i32 = 160;
    const STRENGTH_1: i32 = 161;
    let doses = [
        "Super strength(4)",
        "Super strength(3)",
        "Super strength(2)",
        "Super strength(1)",
    ];
    let proj = site(json!({
        "keyItem": null,
        "foodName": "Lobster",
        "keepExtra": doses,
        "flasks": [{ "flask": "Super strength(4)", "doses": doses, "want": 2 }],
    }));
    let rows = [
        row(STRENGTH_2, 1, "Super strength(2)", 0, false),
        noted_row(NOTED_STRENGTH_2, 3, "Super strength(2)"),
    ];
    for partials in [rows.clone(), swapped(&rows)] {
        let mut pack = partials.to_vec();
        pack.push(row(STRENGTH_1, 1, "Super strength(1)", 0, false));
        let mut sim = TripSim::new(pack, vec![row(2440, 4, "Super strength(4)", 0, false)]);
        let done = run_trip(&proj, &mut sim);
        assert_eq!(done, Some(true), "{:?}", sim.trace);
        assert_eq!(sim.deposited, vec![NOTED_STRENGTH_2], "{:?}", sim.trace);
        assert_eq!(sim.held(STRENGTH_2), 1, "{:?}", sim.trace);
        assert_eq!(sim.held(STRENGTH_1), 1, "{:?}", sim.trace);
        assert_eq!(
            sim.held(2440),
            0,
            "two kept partials meet want 2: {:?}",
            sim.trace
        );
    }
}
