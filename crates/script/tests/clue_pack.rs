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
    assert_eq!(
        value["noRoomKeys"],
        serde_json::json!(["error", "ok"]),
        "{value:?}"
    );
    for key in [
        "omitted", "zeroWant", "held", "capped", "less", "room", "full",
    ] {
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
    for key in [
        "emptyWeapon",
        "blankWeapon",
        "upperAlias",
        "blankAlias",
        "unknownAlias",
    ] {
        assert_eq!(value[key]["ok"], true, "{key} {value:?}");
    }
    assert_eq!(
        value["emptyWeapon"]["value"]["weaponNeeded"], false,
        "{value:?}"
    );
    // `" "` is not `""`: nothing is trimmed.
    assert_eq!(
        value["blankWeapon"]["value"]["weaponNeeded"], true,
        "{value:?}"
    );
    // `_HARD_` is not `_hard_`, and an unrecognised alias is the easy minimum.
    assert_eq!(value["upperAlias"]["value"]["rewardSlots"], 4, "{value:?}");
    assert_eq!(value["blankAlias"]["value"]["rewardSlots"], 4, "{value:?}");
    assert_eq!(
        value["unknownAlias"]["value"]["rewardSlots"], 4,
        "{value:?}"
    );
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

#[test]
fn v2_clue_hard_kit_status_is_the_frozen_first_failure() {
    let src = r#"
export const apiVersion = 2;
function kit(over) {
  return Object.assign({
    attack: 60,
    lostCity: true,
    items: [{ id: 1231, count: 1 }, { id: 185, count: 1 }, { id: 385, count: 15 }],
  }, over || {});
}
export function tick(api) {
  const ready = api.clue.hardKit(kit());
  globalThis.__probe = JSON.stringify({
    ok: ready.ok,
    then: typeof ready.then,
    status: ready.value && ready.value.status,
    keys: ready.value ? Object.keys(ready.value) : 'no-value',
    error: ready.error,
    type: typeof api.clue.hardKit,
    // First failure wins: attack, then Lost City, then the kit.
    lowAttack: api.clue.hardKit(kit({ attack: 59 })),
    lowAttackEmpty: api.clue.hardKit(kit({ attack: 59, lostCity: false, items: [] })),
    noLostCity: api.clue.hardKit(kit({ lostCity: false, items: [] })),
    zeroAttack: api.clue.hardKit(kit({ attack: 0 })),
    empty: api.clue.hardKit({ attack: 60, lostCity: true, items: [] }),
    // A DDS id with no charge is not a dagger we hold, and a dragon longsword
    // is not a DDS at all.
    chargedOut: api.clue.hardKit(kit({ items: [{ id: 1231, count: 0 }, { id: 185, count: 1 }, { id: 385, count: 15 }] })),
    longsword: api.clue.hardKit(kit({ items: [{ id: 1305, count: 1 }, { id: 185, count: 1 }, { id: 385, count: 15 }] })),
    // The unpoisoned local DDS is enough, and ordinary antipoison is not
    // superantipoison.
    unpoisoned: api.clue.hardKit(kit({ items: [{ id: 1215, count: 1 }, { id: 185, count: 1 }, { id: 385, count: 15 }] })),
    antipoison: api.clue.hardKit(kit({ items: [{ id: 1231, count: 1 }, { id: 2446, count: 4 }, { id: 385, count: 15 }] })),
    // One dose is enough, whatever its size: the ready kit with each
    // superantipoison size swapped in.
    dose2448: api.clue.hardKit(kit({ items: [{ id: 1231, count: 1 }, { id: 2448, count: 1 }, { id: 385, count: 15 }] })),
    dose181: api.clue.hardKit(kit({ items: [{ id: 1231, count: 1 }, { id: 181, count: 1 }, { id: 385, count: 15 }] })),
    dose183: api.clue.hardKit(kit({ items: [{ id: 1231, count: 1 }, { id: 183, count: 1 }, { id: 385, count: 15 }] })),
    dose185: api.clue.hardKit(kit({ items: [{ id: 1231, count: 1 }, { id: 185, count: 1 }, { id: 385, count: 15 }] })),
    // An id that matches nothing is a miss, not an argument error.
    otherId: api.clue.hardKit(kit({ items: [{ id: -1231, count: 1 }, { id: 185, count: 1 }, { id: 385, count: 15 }] })),
    fourteen: api.clue.hardKit(kit({ items: [{ id: 1231, count: 1 }, { id: 185, count: 1 }, { id: 385, count: 14 }] })),
    split: api.clue.hardKit(kit({ items: [{ id: 1231, count: 1 }, { id: 185, count: 1 }, { id: 385, count: 7 }, { id: 385, count: 8 }] })),
    otherFood: api.clue.hardKit(kit({ items: [{ id: 1231, count: 1 }, { id: 185, count: 1 }, { id: 391, count: 30 }] })),
    // Extra keys are ignored, on the input and on an item: a worn flag is not
    // an input, and includeBank is neither an input nor a bank read.
    extras: api.clue.hardKit(kit({
      includeBank: true, snapshot: {}, questStatus: 'complete',
      items: [
        Object.assign({ id: 1231, count: 1 }, { slot: 3, worn: true, name: 'Dragon dagger(p)' }),
        Object.assign({ id: 185, count: 1 }, { slot: 4 }),
        Object.assign({ id: 385, count: 15 }, { slot: 5 }),
      ],
    })),
  });
}
"#;
    let value = probe(src);
    assert_eq!(value["ok"], true, "{value:?}");
    assert_eq!(value["then"], "undefined", "{value:?}");
    assert_eq!(value["status"], "ready", "{value:?}");
    assert_eq!(value["keys"], serde_json::json!(["status"]), "{value:?}");
    assert!(value.get("error").is_none(), "{value:?}");
    assert_eq!(value["type"], "function", "{value:?}");
    for key in ["lowAttack", "lowAttackEmpty", "zeroAttack"] {
        assert_eq!(value[key]["error"], "attack", "{key} {value:?}");
    }
    assert_eq!(value["noLostCity"]["error"], "lost-city", "{value:?}");
    for key in ["empty", "chargedOut", "longsword", "otherId"] {
        assert_eq!(value[key]["error"], "dds", "{key} {value:?}");
    }
    assert_eq!(value["antipoison"]["error"], "superantipoison", "{value:?}");
    for key in ["fourteen", "otherFood"] {
        assert_eq!(value[key]["error"], "sharks", "{key} {value:?}");
    }
    // A failure is the whole result, with no value and no published sum.
    for key in ["lowAttack", "noLostCity", "chargedOut", "fourteen"] {
        assert_eq!(value[key]["ok"], false, "{key} {value:?}");
        assert!(value[key].get("value").is_none(), "{key} {value:?}");
        let mut keys = value[key]
            .as_object()
            .map(|row| row.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        keys.sort();
        assert_eq!(
            keys,
            vec!["error".to_string(), "ok".to_string()],
            "{key} {value:?}"
        );
    }
    for key in [
        "unpoisoned",
        "split",
        "extras",
        "dose2448",
        "dose181",
        "dose183",
        "dose185",
    ] {
        assert_eq!(value[key]["ok"], true, "{key} {value:?}");
        assert_eq!(value[key]["value"]["status"], "ready", "{key} {value:?}");
    }
}

#[test]
fn v2_clue_hard_kit_widens_the_dose_and_shark_sums() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const dose = api.clue.hardKit({
    attack: 60, lostCity: true,
    items: [
      { id: 1231, count: 1 },
      { id: 2448, count: 1073741824 },
      { id: 385, count: 15 },
    ],
  });
  const sharks = api.clue.hardKit({
    attack: 60, lostCity: true,
    items: [
      { id: 1231, count: 1 },
      { id: 185, count: 1 },
      { id: 385, count: 2147483647 },
      { id: 385, count: 2147483647 },
    ],
  });
  globalThis.__probe = JSON.stringify({
    doseOk: dose.ok,
    doseStatus: dose.value && dose.value.status,
    doseError: dose.error,
    sharkOk: sharks.ok,
    sharkStatus: sharks.value && sharks.value.status,
    sharkError: sharks.error,
  });
}
"#;
    let value = probe(src);
    // 1073741824 doses is 4294967296, not a wrapped 0.
    assert_eq!(value["doseOk"], true, "{value:?}");
    assert_eq!(value["doseStatus"], "ready", "{value:?}");
    assert!(value.get("doseError").is_none(), "{value:?}");
    // Two i32::MAX stacks are 4294967294, not a wrapped -2.
    assert_eq!(value["sharkOk"], true, "{value:?}");
    assert_eq!(value["sharkStatus"], "ready", "{value:?}");
    assert!(value.get("sharkError").is_none(), "{value:?}");
}

#[test]
fn v2_clue_hard_kit_requires_every_field_and_rejects_a_converted_one() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = JSON.stringify({
    omitted: api.clue.hardKit(),
    undefinedInput: api.clue.hardKit(undefined),
    nullArg: api.clue.hardKit(null),
    array: api.clue.hardKit([]),
    stringArg: api.clue.hardKit('kit'),
    numberArg: api.clue.hardKit(60),
    // An omitted field is not the omitted default: this is invalid-args, not
    // the status attack and not the status lost-city.
    noAttack: api.clue.hardKit({ lostCity: true, items: [] }),
    noLostCity: api.clue.hardKit({ attack: 60, items: [] }),
    noItems: api.clue.hardKit({ attack: 60, lostCity: true }),
    nullAttack: api.clue.hardKit({ attack: null, lostCity: true, items: [] }),
    stringAttack: api.clue.hardKit({ attack: '60', lostCity: true, items: [] }),
    fractionAttack: api.clue.hardKit({ attack: 59.5, lostCity: true, items: [] }),
    negativeAttack: api.clue.hardKit({ attack: -1, lostCity: true, items: [] }),
    negativeZeroAttack: api.clue.hardKit({ attack: -0, lostCity: true, items: [] }),
    aboveI32Attack: api.clue.hardKit({ attack: 2147483648, lostCity: true, items: [] }),
    bigintAttack: api.clue.hardKit({ attack: BigInt(60), lostCity: true, items: [] }),
    boxedAttack: api.clue.hardKit({ attack: new Number(60), lostCity: true, items: [] }),
    // 1 and 0 are not booleans here.
    numberLostCity: api.clue.hardKit({ attack: 60, lostCity: 1, items: [] }),
    zeroLostCity: api.clue.hardKit({ attack: 60, lostCity: 0, items: [] }),
    nullLostCity: api.clue.hardKit({ attack: 60, lostCity: null, items: [] }),
    stringLostCity: api.clue.hardKit({ attack: 60, lostCity: 'true', items: [] }),
    boxedLostCity: api.clue.hardKit({ attack: 60, lostCity: new Boolean(true), items: [] }),
    itemsObject: api.clue.hardKit({ attack: 60, lostCity: true, items: {} }),
    itemsString: api.clue.hardKit({ attack: 60, lostCity: true, items: 'items' }),
    itemsNull: api.clue.hardKit({ attack: 60, lostCity: true, items: null }),
    itemNull: api.clue.hardKit({ attack: 60, lostCity: true, items: [null] }),
    itemNumber: api.clue.hardKit({ attack: 60, lostCity: true, items: [1231] }),
    itemString: api.clue.hardKit({ attack: 60, lostCity: true, items: ['dds'] }),
    itemArray: api.clue.hardKit({ attack: 60, lostCity: true, items: [[1231, 1]] }),
    itemNoId: api.clue.hardKit({ attack: 60, lostCity: true, items: [{ count: 1 }] }),
    itemNoCount: api.clue.hardKit({ attack: 60, lostCity: true, items: [{ id: 1231 }] }),
    itemNullId: api.clue.hardKit({ attack: 60, lostCity: true, items: [{ id: null, count: 1 }] }),
    itemStringId: api.clue.hardKit({ attack: 60, lostCity: true, items: [{ id: '1231', count: 1 }] }),
    itemFractionId: api.clue.hardKit({ attack: 60, lostCity: true, items: [{ id: 1231.5, count: 1 }] }),
    itemAboveI32Id: api.clue.hardKit({ attack: 60, lostCity: true, items: [{ id: 2147483648, count: 1 }] }),
    itemNullCount: api.clue.hardKit({ attack: 60, lostCity: true, items: [{ id: 1231, count: null }] }),
    itemStringCount: api.clue.hardKit({ attack: 60, lostCity: true, items: [{ id: 1231, count: '1' }] }),
    itemFractionCount: api.clue.hardKit({ attack: 60, lostCity: true, items: [{ id: 1231, count: 1.5 }] }),
    itemNegativeCount: api.clue.hardKit({ attack: 60, lostCity: true, items: [{ id: 1231, count: -1 }] }),
    itemNegativeZeroCount: api.clue.hardKit({ attack: 60, lostCity: true, items: [{ id: 1231, count: -0 }] }),
    itemAboveI32Count: api.clue.hardKit({ attack: 60, lostCity: true, items: [{ id: 1231, count: 4294967296 }] }),
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
        "noAttack",
        "noLostCity",
        "noItems",
        "nullAttack",
        "stringAttack",
        "fractionAttack",
        "negativeAttack",
        "negativeZeroAttack",
        "aboveI32Attack",
        "bigintAttack",
        "boxedAttack",
        "numberLostCity",
        "zeroLostCity",
        "nullLostCity",
        "stringLostCity",
        "boxedLostCity",
        "itemsObject",
        "itemsString",
        "itemsNull",
        "itemNull",
        "itemNumber",
        "itemString",
        "itemArray",
        "itemNoId",
        "itemNoCount",
        "itemNullId",
        "itemStringId",
        "itemFractionId",
        "itemAboveI32Id",
        "itemNullCount",
        "itemStringCount",
        "itemFractionCount",
        "itemNegativeCount",
        "itemNegativeZeroCount",
        "itemAboveI32Count",
    ] {
        assert_eq!(value[key]["error"], "invalid-args", "{key} {value:?}");
        assert!(value[key].get("value").is_none(), "{key} {value:?}");
    }
}

#[test]
fn v2_clue_hard_kit_is_not_a_request_op_and_pushes_no_interact() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  let requested = null;
  try { api.request({ op: 'hardKit' }); requested = 'ok'; }
  catch (e) { requested = String(e && (e.message || e)); }
  globalThis.__probe = JSON.stringify({
    kit: api.clue.hardKit({
      attack: 60, lostCity: true,
      items: [{ id: 1231, count: 1 }, { id: 185, count: 1 }, { id: 385, count: 15 }],
    }),
    keys: api.clue && Object.keys(api.clue),
    begin: typeof api.clue.begin,
    next: typeof api.clue.next,
    challengeAnswer: typeof api.clue.challengeAnswer,
    deposit: typeof api.clue.deposit,
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
    assert_eq!(value["kit"]["ok"], true, "{value:?}");
    assert_eq!(value["kit"]["value"]["status"], "ready", "{value:?}");
    assert_eq!(
        value["keys"],
        serde_json::json!(["row", "heldStep", "packPlan", "hardKit", "keep", "begin", "next"]),
        "{value:?}"
    );
    for key in ["begin", "next"] {
        assert_eq!(value[key], "function", "{key} {value:?}");
    }
    for key in ["challengeAnswer", "deposit"] {
        assert_eq!(value[key], "undefined", "{key} {value:?}");
    }
    assert!(
        value["requested"]
            .as_str()
            .unwrap_or("")
            .contains("not impl"),
        "{value:?}"
    );
    assert!(
        interacts.is_empty(),
        "hardKit must not push interact: {interacts:?}"
    );
}

#[test]
fn example_clue_hard_kit_v2_is_one_read_only_kit_call() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("clue_hard_kit_v2.ts");
    let src = std::fs::read_to_string(&path).expect("example source");
    assert!(!src.contains("request("));
    assert!(!src.contains("h.interact"));
    assert!(!src.contains("api.snapshot"));
    assert!(!src.contains("questStatus"));
    assert!(!src.contains("includeBank"));
    assert!(!src.contains("challengeAnswer"));
    assert!(!src.contains("deposit"));
    assert_eq!(src.matches("api.clue").count(), 1);
    assert_eq!(src.matches("hardKit").count(), 1);
    let js = script::transpile_ts(&src).expect("transpile clue_hard_kit_v2.ts");
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
        .find(|line| line.contains("\"status\""))
        .unwrap_or_else(|| panic!("example logged a kit status; logs={logs:?}"));
    let kit: serde_json::Value = serde_json::from_str(last).unwrap();
    assert_eq!(kit["ok"], true, "{kit:?}");
    assert_eq!(kit["value"]["status"], "ready", "{kit:?}");
}

#[test]
fn v2_clue_keep_is_a_sync_helper_result_over_caller_names() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  const spade = api.clue.keep({ name: 'Spade' });
  const shark = api.clue.keep({ name: 'Shark' });
  globalThis.__probe = JSON.stringify({
    ok: spade.ok,
    then: typeof spade.then,
    keep: spade.value && spade.value.keep,
    keys: spade.value ? Object.keys(spade.value) : 'no-value',
    error: spade.error,
    type: typeof api.clue.keep,
    // The six frozen identities, after the ASCII lower and with no trim.
    coins: api.clue.keep({ name: 'COINS' }).value.keep,
    shantay: api.clue.keep({ name: 'Shantay pass' }).value.keep,
    trio: api.clue.keep({ name: 'sextant' }).value.keep
      && api.clue.keep({ name: 'Watch' }).value.keep
      && api.clue.keep({ name: 'chart' }).value.keep,
    padded: api.clue.keep({ name: ' spade' }).value.keep,
    // The frozen substrings, wherever they land, and the frozen pirate
    // casket with them.
    scroll: api.clue.keep({ name: 'Clue scroll' }).value.keep,
    trail: api.clue.keep({ name: 'trail_clue_hard_sextant028' }).value.keep,
    pirate: api.clue.keep({ name: 'Casket' }).value.keep
      && api.clue.keep({ name: 'Pirate casket' }).value.keep,
    // Food is never implied, and a miss is ok false rather than an error.
    sharkOk: shark.ok,
    shark: shark.value && shark.value.keep,
    sharkError: shark.error,
    blank: api.clue.keep({ name: '' }).value.keep,
    // Extra is additive and compared by ASCII equality: it cannot widen the
    // match, and it cannot un-keep a constant or a substring hit.
    extra: api.clue.keep({ name: 'Rune scimitar', extra: ['rune scimitar'] }).value.keep,
    extraUpper: api.clue.keep({ name: 'RUNE SCIMITAR', extra: ['rune scimitar'] }).value.keep,
    extraWide: api.clue.keep({ name: 'Rune scimitar', extra: ['Rune'] }).value.keep,
    extraMiss: api.clue.keep({ name: 'Rune scimitar', extra: [] }).value.keep,
    extraCannotUnkeep: api.clue.keep({ name: 'Clue scroll', extra: ['Shark'] }).value.keep,
    // Extra input keys are ignored: they are not an inventory or bank read.
    extraKeys: api.clue.keep({
      name: 'Coins', extra: ['Shark'], snapshot: {}, items: [], bank: { coins: 1 },
    }).value.keep,
    blankEntry: api.clue.keep({ name: '', extra: [''] }).value.keep,
  });
}
"#;
    let value = probe(src);
    assert_eq!(value["ok"], true, "{value:?}");
    assert_eq!(value["then"], "undefined", "{value:?}");
    assert_eq!(value["keep"], true, "{value:?}");
    assert_eq!(value["keys"], serde_json::json!(["keep"]), "{value:?}");
    assert!(value.get("error").is_none(), "{value:?}");
    assert_eq!(value["type"], "function", "{value:?}");
    for key in ["coins", "shantay", "trio", "scroll", "trail", "pirate"] {
        assert_eq!(value[key], true, "{key} {value:?}");
    }
    // Nothing is trimmed, so the padded name is not the identity.
    assert_eq!(value["padded"], false, "{value:?}");
    assert_eq!(value["sharkOk"], true, "{value:?}");
    assert_eq!(value["shark"], false, "{value:?}");
    assert!(value.get("sharkError").is_none(), "{value:?}");
    assert_eq!(value["blank"], false, "{value:?}");
    for key in [
        "extra",
        "extraUpper",
        "extraCannotUnkeep",
        "extraKeys",
        "blankEntry",
    ] {
        assert_eq!(value[key], true, "{key} {value:?}");
    }
    for key in ["extraWide", "extraMiss"] {
        assert_eq!(value[key], false, "{key} {value:?}");
    }
}

#[test]
fn v2_clue_keep_requires_a_name_and_an_all_string_extra() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  globalThis.__probe = JSON.stringify({
    omitted: api.clue.keep(),
    undefinedInput: api.clue.keep(undefined),
    nullArg: api.clue.keep(null),
    array: api.clue.keep([]),
    stringArg: api.clue.keep('Spade'),
    numberArg: api.clue.keep(1),
    // name is required: a missing, null, or converted one is invalid-args
    // rather than an empty or stringified name.
    empty: api.clue.keep({}),
    nullName: api.clue.keep({ name: null }),
    undefinedName: api.clue.keep({ name: undefined }),
    numberName: api.clue.keep({ name: 1 }),
    boolName: api.clue.keep({ name: true }),
    boxedName: api.clue.keep({ name: new String('Spade') }),
    arrayName: api.clue.keep({ name: ['Spade'] }),
    // extra is optional, but a present null, non-array, or non-string
    // element is invalid-args.
    extraNull: api.clue.keep({ name: 'Spade', extra: null }),
    extraString: api.clue.keep({ name: 'Spade', extra: 'Shark' }),
    extraNumber: api.clue.keep({ name: 'Spade', extra: 3 }),
    extraObject: api.clue.keep({ name: 'Spade', extra: {} }),
    extraBool: api.clue.keep({ name: 'Spade', extra: true }),
    extraNumberElement: api.clue.keep({ name: 'Spade', extra: [1] }),
    extraNullElement: api.clue.keep({ name: 'Spade', extra: [null] }),
    extraUndefinedElement: api.clue.keep({ name: 'Spade', extra: [undefined] }),
    extraBoxedElement: api.clue.keep({ name: 'Spade', extra: [new String('Shark')] }),
    extraArrayElement: api.clue.keep({ name: 'Spade', extra: [['Shark']] }),
    extraObjectElement: api.clue.keep({ name: 'Spade', extra: [{ name: 'Shark' }] }),
    extraSparse: api.clue.keep({ name: 'Spade', extra: new Array(1) }),
    // The present shapes that are not errors.
    blankName: api.clue.keep({ name: '' }),
    emptyExtra: api.clue.keep({ name: 'spade', extra: [] }),
    blankEntry: api.clue.keep({ name: '', extra: [''] }),
    extraOnly: api.clue.keep({ name: 'Rope', extra: ['Rope'] }),
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
        "empty",
        "nullName",
        "undefinedName",
        "numberName",
        "boolName",
        "boxedName",
        "arrayName",
        "extraNull",
        "extraString",
        "extraNumber",
        "extraObject",
        "extraBool",
        "extraNumberElement",
        "extraNullElement",
        "extraUndefinedElement",
        "extraBoxedElement",
        "extraArrayElement",
        "extraObjectElement",
        "extraSparse",
    ] {
        assert_eq!(value[key]["error"], "invalid-args", "{key} {value:?}");
        assert!(value[key].get("value").is_none(), "{key} {value:?}");
    }
    assert_eq!(value["blankName"]["ok"], true, "{value:?}");
    assert_eq!(value["blankName"]["value"]["keep"], false, "{value:?}");
    for key in ["emptyExtra", "blankEntry", "extraOnly"] {
        assert_eq!(value[key]["ok"], true, "{key} {value:?}");
        assert_eq!(value[key]["value"]["keep"], true, "{key} {value:?}");
    }
}

#[test]
fn v2_clue_keep_is_not_a_request_op_and_pushes_no_interact() {
    let src = r#"
export const apiVersion = 2;
export function tick(api) {
  let requested = null;
  try { api.request({ op: 'keep' }); requested = 'ok'; }
  catch (e) { requested = String(e && (e.message || e)); }
  globalThis.__probe = JSON.stringify({
    kept: api.clue.keep({ name: 'Clue scroll' }),
    begin: typeof api.clue.begin,
    next: typeof api.clue.next,
    challengeAnswer: typeof api.clue.challengeAnswer,
    deposit: typeof api.clue.deposit,
    retry: typeof api.clue.retry,
    noteDeath: typeof api.clue.noteDeath,
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
    assert_eq!(value["kept"]["ok"], true, "{value:?}");
    assert_eq!(value["kept"]["value"]["keep"], true, "{value:?}");
    for key in ["begin", "next"] {
        assert_eq!(value[key], "function", "{key} {value:?}");
    }
    for key in ["challengeAnswer", "deposit", "retry", "noteDeath"] {
        assert_eq!(value[key], "undefined", "{key} {value:?}");
    }
    assert!(
        value["requested"]
            .as_str()
            .unwrap_or("")
            .contains("not impl"),
        "{value:?}"
    );
    assert!(
        interacts.is_empty(),
        "keep must not push interact: {interacts:?}"
    );
}

#[test]
fn example_clue_keep_v2_is_one_read_only_keep_call() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("clue_keep_v2.ts");
    let src = std::fs::read_to_string(&path).expect("example source");
    assert!(!src.contains("request("));
    assert!(!src.contains("h.interact"));
    assert!(!src.contains("api.snapshot"));
    assert!(!src.contains("deposit"));
    assert!(!src.contains("challengeAnswer"));
    assert_eq!(src.matches("api.clue").count(), 1);
    assert_eq!(src.matches("api.clue.keep(").count(), 1);
    let js = script::transpile_ts(&src).expect("transpile clue_keep_v2.ts");
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
        .find(|line| line.contains("\"keep\""))
        .unwrap_or_else(|| panic!("example logged a keep result; logs={logs:?}"));
    let keep: serde_json::Value = serde_json::from_str(last).unwrap();
    assert_eq!(keep["ok"], true, "{keep:?}");
    assert_eq!(keep["value"]["keep"], true, "{keep:?}");
}
