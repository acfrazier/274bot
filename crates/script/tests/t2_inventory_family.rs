//! Served T2 inventory-family behavior through real LoadIsolate V8 instances.

use script::isolate_fb::{
    encode_snapshot_with_native, ItemRowInput, NativeFactsInput, SnapshotInput,
};
use script::load::{LoadIsolate, LoadShape};
use script::shim::InteractReq;
use serde_json::{json, Value};

mod common;

const API_SURFACE: &str = r#"
import { Inventory, InvItem, backpackSnapshots, backpackCapacity } from '../../api/inventory/Inventory.js';
import { Equipment } from '../../api/equipment/Equipment.js';
import {
    matchesAny, countMatching, slotsMatching, shouldBank, shouldRestock, shouldEat, shouldPanic,
} from '../../api/inventory/packRules.js';

globalThis.__readInventory = () => {
    const items = Inventory.items();
    const first = Inventory.first('APPLE');
    const actions = first.actions();
    const freshActions = first.actions() !== actions;
    actions.push('caller-only');
    const manual = new InvItem({
        name: 'Manual', count: 4, id: 900, slot: 8, noted: true, cert: 17, ops: [],
    });
    globalThis.__manualInvItem = manual;
    return {
        rootType: first instanceof InvItem,
        itemsType: items[0] instanceof InvItem,
        used: Inventory.used(),
        count: Inventory.count('apple'),
        countById: Inventory.countById('10'),
        firstName: first.name,
        firstSlot: first.slot,
        firstNoted: first.noted,
        firstCert: first.cert,
        secondNoted: items[1].noted,
        secondCert: items[1].cert,
        freshActions,
        actionsAfterMutation: first.actions(),
        manualType: manual instanceof InvItem,
        manualCert: manual.cert,
        contains: Inventory.contains('APPLE'),
        isFull: Inventory.isFull(),
        free: Inventory.free(),
        backpackCapacity: backpackCapacity(),
        backpackKeys: Object.keys(backpackSnapshots()[0]),
        backpackFirst: backpackSnapshots()[0],
        equipmentItems: Equipment.items(),
        equipmentContains: Equipment.contains('BRONZE SWORD'),
    };
};

globalThis.__tryBackpack = () => {
    try { return backpackSnapshots(); }
    catch (error) { return String(error && error.message || error); }
};
globalThis.__backpackCapacity = () => backpackCapacity();
globalThis.__countByIdValue = () => {
    try { return Inventory.countById(10); }
    catch (error) { return String(error && error.message || error); }
};
globalThis.__countApple = () => Inventory.count('Apple');
globalThis.__capacityWithoutHostRead = () => {
    let reads = 0;
    Object.defineProperty(globalThis.__rs2b0t_host, 'invSize', {
        configurable: true,
        get() { reads++; return 3; },
    });
    const capacity = backpackCapacity();
    delete globalThis.__rs2b0t_host.invSize;
    return { capacity, reads };
};
globalThis.__inventoryCoercion = () => {
    let stringCalls = 0;
    const coercible = { toString() { stringCalls++; return 'APPLE'; } };
    const count = Inventory.count(coercible);
    const marker = {};
    const originalString = globalThis.String;
    let thrownIdentity = false;
    globalThis.String = () => { throw marker; };
    try { Inventory.count('Apple'); }
    catch (error) { thrownIdentity = error === marker; }
    finally { globalThis.String = originalString; }
    return { count, stringCalls, thrownIdentity };
};
globalThis.__inventoryOverrides = () => {
    const originalFirst = Inventory.first;
    const originalUsed = Inventory.used;
    const originalMax = Math.max;
    const containsCalls = [];
    const order = [];
    Inventory.first = () => { containsCalls.push('first'); return {}; };
    const contains = Inventory.contains('ignored');
    Object.defineProperty(globalThis.__rs2b0t_host, 'invSize', {
        configurable: true,
        get() { order.push('size'); return 3; },
    });
    Inventory.used = () => { order.push('used'); return 2; };
    Math.max = () => { order.push('max'); return 42; };
    const isFull = Inventory.isFull();
    const free = Inventory.free();
    Inventory.first = originalFirst;
    Inventory.used = originalUsed;
    Math.max = originalMax;
    delete globalThis.__rs2b0t_host.invSize;
    return { contains, containsCalls, isFull, free, order };
};
globalThis.__useTargets = () => {
    const source = Inventory.first('Apple');
    const targetItem = Inventory.items()[1];
    class Npc { constructor() { this.name = 'Goblin'; this.snap = { index: 7, x: 1, z: 2, level: 0 }; } }
    class Loc { constructor() { this.name = 'Door'; this.snap = { x: 3, z: 4, level: 1 }; } }
    class GroundItem { constructor() { this.name = 'Coins'; this.snap = { x: 5, z: 6, level: 0 }; } }
    class Player { constructor() { this.name = 'Rival'; this.snap = { x: 7, z: 8, level: 0 }; } }
    class Other { constructor() { this.name = 'Other'; this.snap = {}; } }
    source.name = 'Mutated after construction';
    const interactResult = source.interact({ toString() { return 'Eat'; } });
    const result = [
        source.useOn(targetItem),
        source.useOn(new Npc()),
        source.useOn(new Loc()),
        source.useOn(new GroundItem()),
        source.useOn(new Player()),
        source.useOn(null),
    ];
    let unsupported = '';
    try { source.useOn(new Other()); }
    catch (error) { unsupported = String(error && error.message || error); }
    return { interactResult, result, unsupported };
};
globalThis.__startEquip = (name) => {
    globalThis.__equipResult = 'pending';
    Equipment.equip(name).then((value) => { globalThis.__equipResult = value; });
};
globalThis.__startUnequip = (name) => {
    globalThis.__unequipResult = 'pending';
    Equipment.unequip(name).then((value) => { globalThis.__unequipResult = value; });
};
globalThis.__runFastEquipment = () => {
    Equipment.equip('Apple').then((value) => { globalThis.__alreadyEquipped = value; });
    Equipment.equip('Ghost').then((value) => { globalThis.__missingEquip = value; });
    Equipment.unequip('Ghost').then((value) => { globalThis.__missingUnequip = value; });
};
globalThis.__overrideEquipmentOrder = () => {
    const oldContains = Equipment.contains;
    const calls = [];
    let stringCalls = 0;
    Equipment.contains = () => { calls.push('contains'); return true; };
    const hostile = { toString() { stringCalls++; calls.push('string'); return 'Apple'; } };
    Equipment.equip(hostile).then((value) => {
        Equipment.contains = oldContains;
        globalThis.__overrideEquip = { value, calls, stringCalls };
    });
};
globalThis.__overrideInventoryFirstOrder = () => {
    const oldContains = Equipment.contains;
    const oldFirst = Inventory.first;
    const calls = [];
    let stringCalls = 0;
    Equipment.contains = () => { calls.push('contains'); return false; };
    Inventory.first = () => { calls.push('first'); return null; };
    const hostile = { toString() { stringCalls++; calls.push('string'); return 'Apple'; } };
    Equipment.equip(hostile).then((value) => {
        Equipment.contains = oldContains;
        Inventory.first = oldFirst;
        globalThis.__overrideFirst = { value, calls, stringCalls };
    });
};
globalThis.__packRules = () => {
    const items = [
        null,
        { name: null, count: 99 },
        { name: 'Apple', count: 2 },
        { name: 'Pineapple', count: 3 },
        { name: 'Apple', count: '4' },
        { name: 'Apple', count: NaN },
    ];
    const marker = {};
    let thrownIdentity = false;
    try {
        countMatching([{ get name() { throw marker; }, count: 1 }], ['apple']);
    } catch (error) {
        thrownIdentity = error === marker;
    }
    const someMarker = {};
    let someThrownIdentity = false;
    try {
        matchesAny('Apple', { get some() { throw someMarker; } });
    } catch (error) {
        someThrownIdentity = error === someMarker;
    }
    const overridden = {
        filter() {
            return { reduce(_callback, initial) { return initial + 23; }, length: 7 };
        },
    };
    const coercibleName = { toString() { return 'Big Apple'; } };
    const coerciblePattern = { toString() { return ' apple '; } };
    return {
        nullName: matchesAny(null, ['apple']),
        undefinedName: matchesAny(undefined, ['apple']),
        coercible: matchesAny(coercibleName, [coerciblePattern]),
        falseyPatterns: matchesAny('Apple', 0),
        countIsNaN: Number.isNaN(countMatching(items, ['apple'])),
        slots: slotsMatching(items, ['apple']),
        customFilter: countMatching(overridden, ['apple']),
        shouldBankAt: shouldBank(8, 8, false),
        shouldBankEmptyFull: shouldBank(0, 8, true),
        shouldBankFalseyIdentity: shouldBank(0, 8, 0) === 0,
        shouldBankNaN: shouldBank(NaN, 8, true),
        shouldRestock: shouldRestock('2', 5),
        shouldEat: shouldEat(10, 10, 5, 1),
        shouldPanic: shouldPanic('0.2', 0.25, 0),
        shouldPanicStrictFood: shouldPanic(0.2, 0.25, '0'),
        thrownIdentity,
        someThrownIdentity,
    };
};

export default class T extends LoopingBot { loop() {} }
"#;

fn row<'a>(
    name: Option<&'a str>,
    id: i32,
    count: i32,
    slot: i32,
    component_id: i32,
    noted: bool,
    cert: i32,
    ops: &'a [String],
) -> ItemRowInput<'a> {
    ItemRowInput {
        name,
        id,
        count,
        slot,
        component_id,
        noted,
        cert,
        ops,
    }
}

fn spawn() -> LoadIsolate {
    LoadIsolate::spawn(API_SURFACE.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn post(iso: &LoadIsolate, snapshot: &SnapshotInput<'_>, side_modal_id: Option<i32>) {
    let native = NativeFactsInput {
        side_modal_id,
        ..NativeFactsInput::default()
    };
    iso.post_snapshot(encode_snapshot_with_native(snapshot, native));
    iso.on_game_tick(snapshot.tick);
    let _ = iso.probe("true").unwrap();
}

fn as_use_on(
    request: &InteractReq,
) -> Option<(
    &str,
    &str,
    Option<&str>,
    i32,
    i32,
    i32,
    Option<i32>,
    Option<i32>,
    Option<i32>,
    Option<i32>,
    Option<i32>,
)> {
    match request {
        InteractReq::UseOn {
            name,
            kind,
            target_name,
            x,
            z,
            level,
            index,
            source_item_id,
            source_item_slot,
            target_item_id,
            target_item_slot,
        } => Some((
            name,
            kind,
            target_name.as_deref(),
            *x,
            *z,
            *level,
            *index,
            *source_item_id,
            *source_item_slot,
            *target_item_id,
            *target_item_slot,
        )),
        _ => None,
    }
}

#[test]
fn inventory_equipment_and_snapshot_exports_use_their_intended_views() {
    let iso = spawn();
    let unknown = iso.probe("__tryBackpack()").unwrap();
    assert!(unknown.as_str().is_some_and(|message| message.contains("not impl")));

    let empty: [ItemRowInput<'_>; 0] = [];
    let mut snapshot = common::ingame_snapshot();
    snapshot.tick = 1;
    snapshot.inv = &empty;
    snapshot.inv_size = 0;
    snapshot.bank_open = false;
    snapshot.bank_side = &empty;
    post(&iso, &snapshot, None);
    assert_eq!(iso.probe("__backpackCapacity()").unwrap(), json!(0));
    assert_eq!(iso.probe("__countByIdValue()").unwrap(), json!(0));

    let side_only = [row(Some("Deposit note"), 99, 2, 0, 701, false, -1, &[])];
    snapshot.tick = 2;
    snapshot.bank_open = true;
    snapshot.bank_side = &side_only;
    post(&iso, &snapshot, Some(700));
    assert_eq!(iso.probe("__tryBackpack()[0].name").unwrap(), json!("Deposit note"));
    assert_eq!(iso.probe("__backpackCapacity()").unwrap(), json!(28));
    assert_eq!(
        iso.probe("__capacityWithoutHostRead()").unwrap(),
        json!({ "capacity": 28, "reads": 0 })
    );
    assert_eq!(iso.probe("__countApple()").unwrap(), json!(0));
    iso.probe("delete globalThis.__rs2b0t_host.invSize").unwrap();

    // A bank-open flag without an observed side root is unknown, not an empty backpack.
    snapshot.tick = 3;
    snapshot.bank_side = &side_only;
    post(&iso, &snapshot, Some(-1));
    let unknown_side = iso.probe("__tryBackpack()").unwrap();
    assert!(unknown_side
        .as_str()
        .is_some_and(|message| message.contains("not impl")));

    snapshot.tick = 4;
    snapshot.bank_side = &empty;
    post(&iso, &snapshot, Some(700));
    assert_eq!(iso.probe("__tryBackpack()").unwrap(), json!([]));

    let ops = vec!["Eat".to_string(), "Drop".to_string()];
    let inventory = [
        row(Some("Apple"), 10, 2, 0, 503, false, -1, &ops),
        row(Some("Apple"), 10, 1, 1, 503, true, 17, &ops),
        row(Some("Mystery"), 0, 7, 2, 503, false, -1, &[]),
        row(Some(""), 77, 4, 3, 503, false, -1, &[]),
        row(None, 0, 8, 4, 503, false, -1, &[]),
    ];
    let equipment = [row(Some("Bronze sword"), 130, 1, 0, 600, false, -1, &[])];
    snapshot.tick = 5;
    snapshot.inv = &inventory;
    snapshot.inv_size = 28;
    snapshot.equipment = &equipment;
    snapshot.bank_open = false;
    snapshot.bank_side = &side_only;
    post(&iso, &snapshot, Some(700));

    let result = iso.probe("__readInventory()").unwrap();
    assert_eq!(result["rootType"], json!(true));
    assert_eq!(result["itemsType"], json!(true));
    assert_eq!(result["used"], json!(3));
    assert_eq!(result["count"], json!(3));
    assert_eq!(result["countById"], json!(3));
    assert_eq!(result["firstName"], json!("Apple"));
    assert_eq!(result["firstSlot"], json!(0));
    assert_eq!(result["firstNoted"], json!(false));
    assert_eq!(result["firstCert"], json!(-1));
    assert_eq!(result["secondNoted"], json!(true));
    assert_eq!(result["secondCert"], json!(17));
    assert_eq!(result["freshActions"], json!(true));
    assert_eq!(result["actionsAfterMutation"], json!(["Eat", "Drop"]));
    assert_eq!(result["manualType"], json!(true));
    assert_eq!(result["manualCert"], json!(17));
    assert_eq!(result["contains"], json!(true));
    assert_eq!(result["isFull"], json!(false));
    assert_eq!(result["free"], json!(25));
    assert_eq!(result["backpackCapacity"], json!(28));
    assert_eq!(
        result["backpackKeys"],
        json!(["slot", "id", "name", "count", "noted", "ops", "comId"])
    );
    assert_eq!(result["backpackFirst"]["comId"], json!(503));
    assert_eq!(result["equipmentContains"], json!(true));
    assert_eq!(result["equipmentItems"][0]["id"], json!(130));
    assert_eq!(result["equipmentItems"][0]["count"], json!(1));

    let coercion = iso.probe("__inventoryCoercion()").unwrap();
    assert_eq!(
        coercion,
        json!({ "count": 3, "stringCalls": 1, "thrownIdentity": true })
    );
    assert_eq!(
        iso.probe("__inventoryOverrides()").unwrap(),
        json!({
            "contains": true,
            "containsCalls": ["first"],
            "isFull": false,
            "free": 42,
            "order": ["size", "used", "size", "used", "max"]
        })
    );

    let unidentified = [row(Some("Unidentified"), 0, 1, 0, 503, false, -1, &[])];
    snapshot.tick = 6;
    snapshot.inv = &unidentified;
    post(&iso, &snapshot, Some(700));
    let unavailable_id = iso.probe("__countByIdValue()").unwrap();
    assert!(unavailable_id
        .as_str()
        .is_some_and(|message| message.contains("not impl")));

    // Existing inventory methods must remain on the posted backpack while the bank is open.
    snapshot.tick = 7;
    snapshot.inv = &inventory;
    snapshot.bank_open = true;
    snapshot.bank_side = &side_only;
    post(&iso, &snapshot, Some(700));
    assert_eq!(iso.probe("__countApple()").unwrap(), json!(3));
    assert_eq!(iso.probe("__tryBackpack()[0].name").unwrap(), json!("Deposit note"));
    assert_eq!(iso.probe("__backpackCapacity()").unwrap(), json!(28));

    // A retained side page is not selected after the bank closes.
    snapshot.tick = 8;
    snapshot.bank_open = false;
    post(&iso, &snapshot, Some(-1));
    assert_eq!(iso.probe("__tryBackpack()[0].name").unwrap(), json!("Apple"));
    assert_eq!(iso.probe("__countApple()").unwrap(), json!(3));
    iso.join();
}

#[test]
fn use_on_preserves_item_identity_and_each_supported_target_kind() {
    let iso = spawn();
    let ops = vec!["Eat".to_string()];
    let inventory = [
        row(Some("Apple"), 10, 2, 0, 503, false, -1, &ops),
        row(Some("Apple"), 10, 1, 1, 503, false, -1, &ops),
    ];
    let mut snapshot = common::ingame_snapshot();
    snapshot.tick = 1;
    snapshot.inv = &inventory;
    post(&iso, &snapshot, None);

    let result = iso.probe("__useTargets()").unwrap();
    assert_eq!(result["interactResult"], json!(true));
    assert_eq!(result["result"], json!([true, true, true, true, true, false]));
    assert!(result["unsupported"]
        .as_str()
        .is_some_and(|message| message.contains("not impl")));

    let requests = iso.drain_interacts();
    assert_eq!(requests.len(), 6);
    assert!(matches!(
        &requests[0],
        InteractReq::Held { name, action, .. } if name == "Apple" && action == "Eat"
    ));
    assert_eq!(
        as_use_on(&requests[1]),
        Some(("Apple", "inv", Some("Apple"), 0, 0, 0, None, Some(10), Some(0), Some(10), Some(1)))
    );
    assert_eq!(
        as_use_on(&requests[2]),
        Some(("Apple", "npc", Some("Goblin"), 1, 2, 0, Some(7), Some(10), Some(0), None, None))
    );
    assert_eq!(
        as_use_on(&requests[3]),
        Some(("Apple", "loc", Some("Door"), 3, 4, 1, None, Some(10), Some(0), None, None))
    );
    assert_eq!(
        as_use_on(&requests[4]),
        Some(("Apple", "obj", Some("Coins"), 5, 6, 0, None, Some(10), Some(0), None, None))
    );
    assert_eq!(
        as_use_on(&requests[5]),
        Some(("Apple", "player", Some("Rival"), 7, 8, 0, None, Some(10), Some(0), None, None))
    );
    iso.join();
}

#[test]
fn equipment_waits_keep_queue_order_and_public_method_overrides() {
    let iso = spawn();
    let ops = vec!["Wear".to_string()];
    let inventory = [row(Some("Apple"), 10, 1, 0, 503, false, -1, &ops)];
    let empty: [ItemRowInput<'_>; 0] = [];
    let mut snapshot = common::ingame_snapshot();
    snapshot.tick = 1;
    snapshot.inv = &inventory;
    snapshot.equipment = &empty;
    post(&iso, &snapshot, None);

    iso.probe("__startEquip('Apple')").unwrap();
    assert!(matches!(iso.drain_interacts().as_slice(), [InteractReq::Wear { name }] if name == "Apple"));
    assert_eq!(iso.probe("__equipResult").unwrap(), json!("pending"));

    let equipped = [row(Some("Apple"), 10, 1, 0, 600, false, -1, &[])];
    snapshot.tick = 2;
    snapshot.equipment = &equipped;
    post(&iso, &snapshot, None);
    assert_eq!(iso.probe("__equipResult").unwrap(), json!(true));

    iso.probe("__runFastEquipment()").unwrap();
    let _ = iso.probe("true").unwrap();
    assert_eq!(iso.probe("__alreadyEquipped").unwrap(), json!(true));
    assert_eq!(iso.probe("__missingEquip").unwrap(), json!(false));
    assert_eq!(iso.probe("__missingUnequip").unwrap(), json!(true));
    assert!(iso.drain_interacts().is_empty());

    iso.probe("__startUnequip('Apple')").unwrap();
    assert!(matches!(iso.drain_interacts().as_slice(), [InteractReq::Unequip { name }] if name == "Apple"));
    assert_eq!(iso.probe("__unequipResult").unwrap(), json!("pending"));
    snapshot.tick = 3;
    snapshot.equipment = &empty;
    post(&iso, &snapshot, None);
    assert_eq!(iso.probe("__unequipResult").unwrap(), json!(true));

    iso.probe("__overrideEquipmentOrder()").unwrap();
    let _ = iso.probe("true").unwrap();
    assert_eq!(
        iso.probe("__overrideEquip").unwrap(),
        json!({ "value": true, "calls": ["contains"], "stringCalls": 0 })
    );
    iso.probe("__overrideInventoryFirstOrder()").unwrap();
    let _ = iso.probe("true").unwrap();
    assert_eq!(
        iso.probe("__overrideFirst").unwrap(),
        json!({ "value": false, "calls": ["contains", "first"], "stringCalls": 0 })
    );
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
fn pack_rules_keep_javascript_coercion_short_circuit_and_error_identity() {
    let iso = spawn();
    let mut snapshot = common::ingame_snapshot();
    snapshot.tick = 1;
    post(&iso, &snapshot, None);

    let result: Value = iso.probe("__packRules()").unwrap();
    assert_eq!(result["nullName"], json!(false));
    assert_eq!(result["undefinedName"], json!(false));
    assert_eq!(result["coercible"], json!(true));
    assert_eq!(result["falseyPatterns"], json!(false));
    assert_eq!(result["countIsNaN"], json!(true));
    assert_eq!(result["slots"], json!(4));
    assert_eq!(result["customFilter"], json!(23));
    assert_eq!(result["shouldBankAt"], json!(true));
    assert_eq!(result["shouldBankEmptyFull"], json!(false));
    assert_eq!(result["shouldBankFalseyIdentity"], json!(true));
    assert_eq!(result["shouldBankNaN"], json!(false));
    assert_eq!(result["shouldRestock"], json!(true));
    assert_eq!(result["shouldEat"], json!(false));
    assert_eq!(result["shouldPanic"], json!(true));
    assert_eq!(result["shouldPanicStrictFood"], json!(false));
    assert_eq!(result["thrownIdentity"], json!(true));
    assert_eq!(result["someThrownIdentity"], json!(true));
    iso.join();
}
