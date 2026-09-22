//! Pack targets over the caller's own numbers: pure slot arithmetic, no page
//! and no family. Not a clue machine and not a solve.
use script::{LoadIsolate, LoadShape};

/// One NativeTick v2 card, ticked once with no posted page: the method reads
/// no snapshot, so nothing here posts one.
fn probe(src: &str) -> serde_json::Value {
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    let value = iso.probe("globalThis.__probe").unwrap();
    iso.join();
    serde_json::from_str(value.as_str().unwrap()).unwrap()
}

#[test]
fn v2_clue_pack_plan_is_a_sync_helper_result_over_caller_numbers() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const plan = api.clue.packPlan({
    hostWant: 20,
    freeSlots: 5,
    reserveSlots: 3,
    perCast: 3,
    weaponName: 'Rune scimitar',
    casketAlias: 'trail_clue_hard_sextant001_casket',
  });
  // Extra keys are ignored: they are not an inventory or snapshot override.
  const extra = api.clue.packPlan({
    hostWant: 20, freeSlots: 5, reserveSlots: 3, supported: true, food: 99,
  });
  const bare = api.clue.packPlan({ hostWant: 20, freeSlots: 5, reserveSlots: 3 });
  globalThis.__probe = JSON.stringify({
    ok: plan.ok,
    then: typeof plan.then,
    coord: plan.value && plan.value.coordToolSlots,
    casts: plan.value && plan.value.teleportCasts,
    food: plan.value && plan.value.food,
    runeTarget: plan.value && plan.value.runeTarget,
    weaponNeeded: plan.value && plan.value.weaponNeeded,
    rewardSlots: plan.value && plan.value.rewardSlots,
    supported: plan.value
      ? Object.prototype.hasOwnProperty.call(plan.value, 'supported') : 'no-value',
    keys: plan.value ? Object.keys(plan.value).sort() : 'no-value',
    extraSame: JSON.stringify(extra.value) === JSON.stringify(bare.value),
    extraFood: extra.value && extra.value.food,
    type: typeof api.clue.packPlan,
  });
}
"#;
    let value = probe(src);
    assert_eq!(value["ok"], true, "{value:?}");
    assert_eq!(value["then"], "undefined", "{value:?}");
    assert_eq!(value["coord"], 3, "{value:?}");
    assert_eq!(value["casts"], 20, "{value:?}");
    assert_eq!(value["food"], 2, "{value:?}");
    assert_eq!(value["runeTarget"], 60, "{value:?}");
    assert_eq!(value["weaponNeeded"], true, "{value:?}");
    assert_eq!(value["rewardSlots"], 6, "{value:?}");
    assert_eq!(value["supported"], false, "{value:?}");
    assert_eq!(
        value["keys"],
        serde_json::json!([
            "coordToolSlots",
            "food",
            "rewardSlots",
            "runeTarget",
            "teleportCasts",
            "weaponNeeded",
        ]),
        "{value:?}"
    );
    assert_eq!(value["extraSame"], true, "{value:?}");
    assert_eq!(value["extraFood"], 2, "{value:?}");
    assert_eq!(value["type"], "function", "{value:?}");
}

#[test]
fn v2_clue_pack_plan_widens_past_i32_and_a_huge_room_is_not_no_room() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const wide = api.clue.packPlan({ perCast: 2147483647 });
  const room = api.clue.packPlan({
    hostWant: 20, heldFood: 2147483647, freeSlots: 2147483647, reserveSlots: 0,
  });
  globalThis.__probe = JSON.stringify({
    wideOk: wide.ok,
    wideTarget: wide.value && wide.value.runeTarget,
    roomOk: room.ok,
    roomFood: room.value && room.value.food,
    roomError: room.error,
  });
}
"#;
    let value = probe(src);
    assert_eq!(value["wideOk"], true, "{value:?}");
    // Not a wrapped -20 and not a saturated 2147483647.
    assert_eq!(value["wideTarget"], 42_949_672_940_i64, "{value:?}");
    assert_eq!(value["roomOk"], true, "{value:?}");
    assert_eq!(value["roomFood"], 10, "{value:?}");
    assert!(
        value.get("roomError").is_none(),
        "a huge room is not no-room: {value:?}"
    );
}

#[test]
fn v2_clue_pack_plan_no_room_is_the_whole_result() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const noRoom = api.clue.packPlan({ hostWant: 20, freeSlots: 0 });
  const reserved = api.clue.packPlan({ hostWant: 20, freeSlots: 2, reserveSlots: 3 });
  globalThis.__probe = JSON.stringify({
    noRoom,
    noRoomKeys: Object.keys(noRoom).sort(),
    reserved,
    omitted: api.clue.packPlan({}),
    zeroWant: api.clue.packPlan({ hostWant: 0, freeSlots: 0 }),
    held: api.clue.packPlan({ hostWant: 20, heldFood: 8, freeSlots: 0 }),
    capped: api.clue.packPlan({ hostWant: 20, heldFood: 12, freeSlots: 0 }),
    less: api.clue.packPlan({ hostWant: 6, freeSlots: 22 }),
    room: api.clue.packPlan({ hostWant: 20, freeSlots: 5, reserveSlots: 3 }),
    full: api.clue.packPlan({ hostWant: 20, freeSlots: 22 }),
    zeroRunes: api.clue.packPlan({ perCast: 0 }),
  });
}
"#;
    let value = probe(src);
    for key in ["noRoom", "reserved"] {
        assert_eq!(value[key]["ok"], false, "{key} {value:?}");
        assert_eq!(value[key]["error"], "no-room", "{key} {value:?}");
        assert!(value[key].get("value").is_none(), "{key} {value:?}");
    }
    assert_eq!(value["noRoomKeys"], serde_json::json!(["error", "ok"]), "{value:?}");
    for key in ["omitted", "zeroWant", "held", "capped", "less", "room", "full"] {
        assert_eq!(value[key]["ok"], true, "{key} {value:?}");
        assert!(value[key].get("error").is_none(), "{key} {value:?}");
    }
    assert_eq!(value["omitted"]["value"]["food"], 0, "{value:?}");
    assert_eq!(value["zeroWant"]["value"]["food"], 0, "{value:?}");
    assert_eq!(value["held"]["value"]["food"], 8, "{value:?}");
    assert_eq!(value["capped"]["value"]["food"], 10, "{value:?}");
    assert_eq!(value["less"]["value"]["food"], 6, "{value:?}");
    assert_eq!(value["room"]["value"]["food"], 2, "{value:?}");
    assert_eq!(value["full"]["value"]["food"], 10, "{value:?}");
    // A zero rune target is not no-room, and a rune-only call still reports
    // the food it was not asked for.
    assert_eq!(value["zeroRunes"]["ok"], true, "{value:?}");
    assert_eq!(value["zeroRunes"]["value"]["runeTarget"], 0, "{value:?}");
    assert_eq!(value["zeroRunes"]["value"]["food"], 0, "{value:?}");
}

#[test]
fn v2_clue_pack_plan_absent_and_present_split_invalid_args() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = JSON.stringify({
    omitted: api.clue.packPlan(),
    undefinedInput: api.clue.packPlan(undefined),
    nullArg: api.clue.packPlan(null),
    array: api.clue.packPlan([]),
    stringArg: api.clue.packPlan('hostWant'),
    numberArg: api.clue.packPlan(20),
    nullWant: api.clue.packPlan({ hostWant: null }),
    stringWant: api.clue.packPlan({ hostWant: '20' }),
    fraction: api.clue.packPlan({ hostWant: 20.5 }),
    negative: api.clue.packPlan({ hostWant: -1 }),
    negativeZero: api.clue.packPlan({ hostWant: -0 }),
    aboveI32: api.clue.packPlan({ freeSlots: 2147483648 }),
    bigint: api.clue.packPlan({ perCast: BigInt(3) }),
    boxed: api.clue.packPlan({ perCast: new Number(3) }),
    boolNumber: api.clue.packPlan({ heldFood: true }),
    nullPerCast: api.clue.packPlan({ perCast: null }),
    undefinedField: api.clue.packPlan({ freeSlots: undefined }),
    stringWeapon: api.clue.packPlan({ weaponName: 7 }),
    nullWeapon: api.clue.packPlan({ weaponName: null }),
    boxedWeapon: api.clue.packPlan({ weaponName: new String('Rune scimitar') }),
    nullAlias: api.clue.packPlan({ casketAlias: null }),
    aliasNumber: api.clue.packPlan({ casketAlias: 4 }),
    numberFlag: api.clue.packPlan({ weaponInBackpack: 1 }),
    nullFlag: api.clue.packPlan({ weaponEquipped: null }),
    empty: api.clue.packPlan({}),
    emptyWeapon: api.clue.packPlan({ weaponName: '' }),
    blankWeapon: api.clue.packPlan({ weaponName: ' ' }),
    upperAlias: api.clue.packPlan({ casketAlias: 'trail_clue_HARD_sextant001_casket' }),
    blankAlias: api.clue.packPlan({ casketAlias: '' }),
    unknownAlias: api.clue.packPlan({ casketAlias: 'mystery_casket' }),
  });
}
"#;
    let value = probe(src);
    for key in [
        "omitted",
        "undefinedInput",
        "nullArg",
        "array",
        "stringArg",
        "numberArg",
        "nullWant",
        "stringWant",
        "fraction",
        "negative",
        "negativeZero",
        "aboveI32",
        "bigint",
        "boxed",
        "boolNumber",
        "nullPerCast",
        "undefinedField",
        "stringWeapon",
        "nullWeapon",
        "boxedWeapon",
        "nullAlias",
        "aliasNumber",
        "numberFlag",
        "nullFlag",
    ] {
        assert_eq!(value[key]["error"], "invalid-args", "{key} {value:?}");
        assert!(value[key].get("value").is_none(), "{key} {value:?}");
    }
    // A known key that is present and wrong is invalid-args even when the
    // field could not have changed the output: `weaponInBackpack` without a
    // weapon name is still checked.
    assert_eq!(value["empty"]["ok"], true, "{value:?}");
    assert!(value["empty"].get("weaponNeeded").is_none(), "{value:?}");
    for key in ["emptyWeapon", "blankWeapon", "upperAlias", "blankAlias", "unknownAlias"] {
        assert_eq!(value[key]["ok"], true, "{key} {value:?}");
    }
    assert_eq!(value["emptyWeapon"]["value"]["weaponNeeded"], false, "{value:?}");
    // `" "` is not `""`: nothing is trimmed.
    assert_eq!(value["blankWeapon"]["value"]["weaponNeeded"], true, "{value:?}");
    // `_HARD_` is not `_hard_`, and an unrecognised alias is the easy minimum.
    assert_eq!(value["upperAlias"]["value"]["rewardSlots"], 4, "{value:?}");
    assert_eq!(value["blankAlias"]["value"]["rewardSlots"], 4, "{value:?}");
    assert_eq!(value["unknownAlias"]["value"]["rewardSlots"], 4, "{value:?}");
}

#[test]
fn v2_clue_pack_plan_is_not_a_request_op_and_pushes_no_interact() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  let requested = null;
  try { api.request({ op: 'packPlan' }); requested = 'ok'; }
  catch (e) { requested = String(e && (e.message || e)); }
  globalThis.__probe = JSON.stringify({
    plan: api.clue.packPlan({ hostWant: 6, freeSlots: 22 }),
    requested,
  });
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    let value: serde_json::Value =
        serde_json::from_str(iso.probe("globalThis.__probe").unwrap().as_str().unwrap()).unwrap();
    let interacts = iso.drain_interacts();
    iso.join();
    assert_eq!(value["plan"]["ok"], true, "{value:?}");
    assert_eq!(value["plan"]["value"]["food"], 6, "{value:?}");
    assert!(
        value["requested"]
            .as_str()
            .unwrap_or("")
            .contains("not impl"),
        "{value:?}"
    );
    assert!(
        interacts.is_empty(),
        "packPlan must not push interact: {interacts:?}"
    );
}

#[test]
fn example_clue_pack_v2_is_one_read_only_plan_call() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("clue_pack_v2.ts");
    let src = std::fs::read_to_string(&path).expect("example source");
    assert!(!src.contains("request("));
    assert!(!src.contains("h.interact"));
    assert!(!src.contains("api.snapshot"));
    assert!(!src.contains("challengeAnswer"));
    assert!(!src.contains("deposit"));
    assert_eq!(src.matches("api.clue").count(), 1);
    assert_eq!(src.matches("packPlan").count(), 1);
    let js = script::transpile_ts(&src).expect("transpile clue_pack_v2.ts");
    let iso = LoadIsolate::spawn(js, LoadShape::NativeTick, vec![]).unwrap();
    iso.on_game_tick(1);
    let err = iso
        .probe("globalThis.__rs2b0t_host.lastError || ''")
        .unwrap();
    let logs = iso.drain_logs();
    iso.join();
    assert_eq!(err.as_str().unwrap_or(""), "", "example lastError: {err:?}");
    let last = logs
        .iter()
        .rev()
        .find(|line| line.contains("\"coordToolSlots\""))
        .unwrap_or_else(|| panic!("example logged a plan; logs={logs:?}"));
    let plan: serde_json::Value = serde_json::from_str(last).unwrap();
    assert_eq!(plan["ok"], true, "{plan:?}");
    assert_eq!(plan["value"]["coordToolSlots"], 3, "{plan:?}");
    assert_eq!(plan["value"]["teleportCasts"], 20, "{plan:?}");
    assert_eq!(plan["value"]["food"], 2, "{plan:?}");
    assert_eq!(plan["value"]["runeTarget"], 60, "{plan:?}");
    assert_eq!(plan["value"]["weaponNeeded"], false, "{plan:?}");
    assert_eq!(plan["value"]["rewardSlots"], 6, "{plan:?}");
}
