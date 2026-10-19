#![cfg(feature = "load")]

//! Served T2 inventory-family behavior through real LoadIsolate V8 instances.

use script::isolate_fb::{
    encode_snapshot_with_native, ItemRowInput, NativeFactsInput, SnapshotInput,
};
use script::load::{LoadIsolate, LoadShape};
use script::shim::InteractReq;
use serde_json::{json, Value};

mod common;

const API_SURFACE: &str = r#"
import * as Root from '@rs2b0t/api';
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
globalThis.__tryBackpackCapacity = () => {
    try { return backpackCapacity(); }
    catch (error) { return String(error && error.message || error); }
};
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
globalThis.__invItemClass = InvItem;
globalThis.__inventoryObject = Inventory;
globalThis.__equipmentObject = Equipment;
globalThis.__inventoryRootIdentity = {
    InvItem: Root.InvItem === InvItem,
    Inventory: Root.Inventory === Inventory,
    Equipment: Root.Equipment === Equipment,
    backpackSnapshotsModuleOnly: !('backpackSnapshots' in Root),
    backpackCapacityModuleOnly: !('backpackCapacity' in Root),
};
globalThis.__inventoryRootRead = () => ({
    names: backpackSnapshots().map(row => row.name),
    capacity: backpackCapacity(),
    inventoryCount: Root.Inventory.count('apple'),
});

export default class T extends LoopingBot {
    loop() {
        const work = globalThis.__inventoryWork;
        if (work) {
            globalThis.__inventoryWork = null;
            globalThis.__inventoryWorkResult = work();
        }
    }
}
"#;

fn row<'a>(
    name: Option<&'a str>,
    id: i32,
    count: i32,
    slot: i32,
    component_id: i32,
    (noted, cert): (bool, i32),
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

fn schedule_work(iso: &LoadIsolate, expression: &str) {
    iso.probe(&format!(
        "globalThis.__inventoryWork = () => ({expression}); true"
    ))
    .unwrap();
}

type UseOnFields<'a> = (
    &'a str,
    &'a str,
    Option<&'a str>,
    i32,
    i32,
    i32,
    Option<i32>,
    Option<i32>,
    Option<i32>,
    Option<i32>,
    Option<i32>,
);

fn as_use_on(request: &InteractReq) -> Option<UseOnFields<'_>> {
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
fn backpack_helpers_refuse_an_unposted_bank_open_page() {
    let iso = spawn();
    for (read, name) in [
        ("__tryBackpack()", "backpackSnapshots"),
        ("__tryBackpackCapacity()", "backpackCapacity"),
    ] {
        assert_eq!(
            iso.probe(read).unwrap(),
            json!(format!("not impl: {name}")),
            "an unposted bank-open page must remain unknown for {name}"
        );
    }
    iso.join();
}

#[test]
fn inventory_equipment_and_snapshot_exports_use_their_intended_views() {
    let iso = spawn();
    let unknown = iso.probe("__tryBackpack()").unwrap();
    assert!(unknown
        .as_str()
        .is_some_and(|message| message.contains("not impl")));

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

    let side_only = [row(Some("Deposit note"), 99, 2, 0, 701, (false, -1), &[])];
    snapshot.tick = 2;
    snapshot.bank_open = true;
    snapshot.bank_side = &side_only;
    post(&iso, &snapshot, Some(700));
    assert_eq!(
        iso.probe("__tryBackpack()[0].name").unwrap(),
        json!("Deposit note")
    );
    assert_eq!(iso.probe("__backpackCapacity()").unwrap(), json!(28));
    assert_eq!(
        iso.probe("__capacityWithoutHostRead()").unwrap(),
        json!({ "capacity": 28, "reads": 0 })
    );
    assert_eq!(iso.probe("__countApple()").unwrap(), json!(0));
    iso.probe("delete globalThis.__rs2b0t_host.invSize")
        .unwrap();

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
        row(Some("Apple"), 10, 2, 0, 503, (false, -1), &ops),
        row(Some("Apple"), 10, 1, 1, 503, (true, 17), &ops),
        row(Some("Mystery"), 0, 7, 2, 503, (false, -1), &[]),
        row(Some(""), 77, 4, 3, 503, (false, -1), &[]),
        row(None, 0, 8, 4, 503, (false, -1), &[]),
    ];
    let equipment = [row(Some("Bronze sword"), 130, 1, 0, 600, (false, -1), &[])];
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
            "order": ["size", "size", "used", "size", "size", "used", "max"]
        })
    );

    let unidentified = [row(Some("Unidentified"), 0, 1, 0, 503, (false, -1), &[])];
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
    assert_eq!(
        iso.probe("__tryBackpack()[0].name").unwrap(),
        json!("Deposit note")
    );
    assert_eq!(iso.probe("__backpackCapacity()").unwrap(), json!(28));

    // A retained side page is not selected after the bank closes.
    snapshot.tick = 8;
    snapshot.bank_open = false;
    post(&iso, &snapshot, Some(-1));
    assert_eq!(
        iso.probe("__tryBackpack()[0].name").unwrap(),
        json!("Apple")
    );
    assert_eq!(iso.probe("__countApple()").unwrap(), json!(3));
    iso.join();
}

#[test]
fn use_on_preserves_item_identity_and_each_supported_target_kind() {
    let iso = spawn();
    let ops = vec!["Eat".to_string()];
    let inventory = [
        row(Some("Apple"), 10, 2, 0, 503, (false, -1), &ops),
        row(Some("Apple"), 10, 1, 1, 503, (false, -1), &ops),
    ];
    let mut snapshot = common::ingame_snapshot();
    snapshot.tick = 1;
    snapshot.inv = &inventory;
    post(&iso, &snapshot, None);

    schedule_work(&iso, "__useTargets()");
    snapshot.tick = 2;
    post(&iso, &snapshot, None);
    let result = iso.probe("__inventoryWorkResult").unwrap();
    assert_eq!(result["interactResult"], json!(true));
    assert_eq!(
        result["result"],
        json!([true, true, true, true, true, false])
    );
    assert!(result["unsupported"]
        .as_str()
        .is_some_and(|message| message.contains("not impl")));

    let requests = iso.drain_interacts();
    assert_eq!(requests.len(), 6);
    assert!(matches!(
        &requests[0],
        InteractReq::Held { name, action, .. } if name == "Mutated after construction" && action == "Eat"
    ));
    assert_eq!(
        as_use_on(&requests[1]),
        Some((
            "Mutated after construction",
            "inv",
            Some("Apple"),
            0,
            0,
            0,
            None,
            Some(10),
            Some(0),
            Some(10),
            Some(1)
        ))
    );
    assert_eq!(
        as_use_on(&requests[2]),
        Some((
            "Mutated after construction",
            "npc",
            Some("Goblin"),
            1,
            2,
            0,
            Some(7),
            Some(10),
            Some(0),
            None,
            None
        ))
    );
    assert_eq!(
        as_use_on(&requests[3]),
        Some((
            "Mutated after construction",
            "loc",
            Some("Door"),
            3,
            4,
            1,
            None,
            Some(10),
            Some(0),
            None,
            None
        ))
    );
    assert_eq!(
        as_use_on(&requests[4]),
        Some((
            "Mutated after construction",
            "obj",
            Some("Coins"),
            5,
            6,
            0,
            None,
            Some(10),
            Some(0),
            None,
            None
        ))
    );
    assert_eq!(
        as_use_on(&requests[5]),
        Some((
            "Mutated after construction",
            "player",
            Some("Rival"),
            7,
            8,
            0,
            None,
            Some(10),
            Some(0),
            None,
            None
        ))
    );
    iso.join();
}

#[test]
fn equipment_waits_keep_queue_order_and_public_method_overrides() {
    let iso = spawn();
    let ops = vec!["Wear".to_string()];
    let inventory = [row(Some("Apple"), 10, 1, 0, 503, (false, -1), &ops)];
    let empty: [ItemRowInput<'_>; 0] = [];
    let mut snapshot = common::ingame_snapshot();
    snapshot.tick = 1;
    snapshot.inv = &inventory;
    snapshot.equipment = &empty;
    schedule_work(&iso, "__startEquip('Apple')");
    post(&iso, &snapshot, None);

    assert!(
        matches!(iso.drain_interacts().as_slice(), [InteractReq::Wear { name }] if name == "Apple")
    );
    assert_eq!(iso.probe("__equipResult").unwrap(), json!("pending"));

    let equipped = [row(Some("Apple"), 10, 1, 0, 600, (false, -1), &[])];
    snapshot.tick = 2;
    snapshot.equipment = &equipped;
    post(&iso, &snapshot, None);
    assert_eq!(iso.probe("__equipResult").unwrap(), json!(true));

    schedule_work(&iso, "__runFastEquipment()");
    snapshot.tick = 3;
    post(&iso, &snapshot, None);
    assert_eq!(iso.probe("__alreadyEquipped").unwrap(), json!(true));
    assert_eq!(iso.probe("__missingEquip").unwrap(), json!(false));
    assert_eq!(iso.probe("__missingUnequip").unwrap(), json!(true));
    assert!(iso.drain_interacts().is_empty());

    schedule_work(&iso, "__startUnequip('Apple')");
    snapshot.tick = 4;
    post(&iso, &snapshot, None);
    assert!(
        matches!(iso.drain_interacts().as_slice(), [InteractReq::Unequip { name }] if name == "Apple")
    );
    assert_eq!(iso.probe("__unequipResult").unwrap(), json!("pending"));
    snapshot.tick = 5;
    snapshot.equipment = &empty;
    post(&iso, &snapshot, None);
    assert_eq!(iso.probe("__unequipResult").unwrap(), json!(true));

    schedule_work(&iso, "__overrideEquipmentOrder()");
    snapshot.tick = 6;
    post(&iso, &snapshot, None);
    assert_eq!(
        iso.probe("__overrideEquip").unwrap(),
        json!({ "value": true, "calls": ["contains"], "stringCalls": 0 })
    );
    schedule_work(&iso, "__overrideInventoryFirstOrder()");
    snapshot.tick = 7;
    post(&iso, &snapshot, None);
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

#[test]
fn item_constructor_keeps_getter_setter_metadata_and_strict_write_order() {
    let iso = spawn();
    let result = iso
        .probe(
            r#"(() => {
        const InvItem = globalThis.__invItemClass;
        const events = [];
        const written = {};
        let ids = 0;
        let slots = 0;
        const row = {
            get id() { events.push('id.get'); return 9 + ++ids; },
            get slot() { events.push('slot.get'); return 1 + ++slots; },
            get name() { events.push('name.get'); return 'Manual'; },
            get count() { events.push('count.get'); return 7; },
            get noted() { events.push('noted.get'); return 1; },
            get cert() { events.push('cert.get'); return null; },
        };
        class WithSetters extends InvItem {}
        for (const field of ['name', 'count', 'id', 'slot', 'noted', 'cert']) {
            Object.defineProperty(WithSetters.prototype, field, {
                set(value) { events.push(field + '.set'); written[field] = value; },
            });
        }
        const descriptor = Object.getOwnPropertyDescriptor(Number, 'isInteger');
        const original = descriptor.value;
        let integerReceivers = true;
        Object.defineProperty(Number, 'isInteger', {
            configurable: true,
            get() {
                events.push('integer.get');
                return function(value) {
                    events.push('integer.call');
                    integerReceivers &&= this === Number;
                    return original(value);
                };
            },
        });
        let item;
        try { item = new WithSetters(row); }
        finally { Object.defineProperty(Number, 'isInteger', descriptor); }
        const keys = Object.getOwnPropertySymbols(item);
        const identity = keys.map(key => item[key]).find(value => value !== row);
        const metadata = keys.length === 2
            && keys.some(key => item[key] === row)
            && keys.every(key => {
                const property = Object.getOwnPropertyDescriptor(item, key);
                return property.enumerable && property.writable && property.configurable;
            });
        class ReadOnly extends InvItem {}
        Object.defineProperty(ReadOnly.prototype, 'name', {
            value: 'read only', writable: false,
        });
        let strictWrite = false;
        try { new ReadOnly({name: 'new'}); }
        catch (error) { strictWrite = error instanceof TypeError; }
        const sentinel = {};
        class ThrowingSetter extends InvItem {}
        Object.defineProperty(ThrowingSetter.prototype, 'count', {
            set() { throw sentinel; },
        });
        let setterIdentity = false;
        try { new ThrowingSetter({name: 'new', count: 1}); }
        catch (error) { setterIdentity = error === sentinel; }
        let nullThrows = false;
        try { new InvItem(null); }
        catch (error) { nullThrows = error instanceof TypeError; }
        return {
            events, written, integerReceivers, metadata, strictWrite,
            setterIdentity, nullThrows, identity,
        };
    })()"#,
        )
        .unwrap();
    assert_eq!(
        result["events"],
        json!([
            "integer.get",
            "id.get",
            "integer.call",
            "integer.get",
            "slot.get",
            "integer.call",
            "slot.get",
            "name.get",
            "name.set",
            "count.get",
            "count.set",
            "id.get",
            "id.set",
            "slot.get",
            "slot.set",
            "noted.get",
            "noted.set",
            "cert.get",
            "cert.set",
            "id.get",
            "slot.get",
        ])
    );
    assert_eq!(
        result["written"],
        json!({
            "name": "Manual", "count": 7, "id": 11, "slot": 4, "noted": false, "cert": -1,
        })
    );
    assert_eq!(result["identity"], json!({"id": 12, "slot": 5}));
    for key in [
        "integerReceivers",
        "metadata",
        "strictWrite",
        "setterIdentity",
        "nullThrows",
    ] {
        assert_eq!(result[key], true, "{key}: {result}");
    }
    iso.join();
}

#[test]
fn item_actions_and_contains_keep_overridable_callees_and_original_rows() {
    let iso = spawn();
    let result = iso.probe(r#"(() => {
        const InvItem = globalThis.__invItemClass;
        const Inventory = globalThis.__inventoryObject;
        const events = [];
        const token = {};
        let opsReads = 0;
        let receivers = true;
        const secondOps = {
            get slice() {
                events.push('slice.get');
                return function() {
                    events.push('slice.call');
                    receivers &&= this === secondOps;
                    return token;
                };
            },
        };
        const row = {
            get ops() {
                events.push('ops.' + ++opsReads);
                return opsReads === 1 ? [] : secondOps;
            },
        };
        const item = new InvItem(row);
        const descriptor = Object.getOwnPropertyDescriptor(Array, 'isArray');
        const original = descriptor.value;
        Object.defineProperty(Array, 'isArray', {
            configurable: true,
            get() {
                events.push('isArray.get');
                return function(value) {
                    events.push('isArray.call');
                    receivers &&= this === Array;
                    return original(value);
                };
            },
        });
        let actionsIdentity;
        try { actionsIdentity = item.actions() === token; }
        finally { Object.defineProperty(Array, 'isArray', descriptor); }
        const emptyItem = new InvItem({ops: false});
        const emptyA = emptyItem.actions();
        const emptyB = emptyItem.actions();
        const freshEmpty = Array.isArray(emptyA) && emptyA.length === 0 && emptyA !== emptyB;
        const retained = {ops: ['first']};
        const retainedItem = new InvItem(retained);
        retained.ops = ['second'];
        const retainedRow = retainedItem.actions()[0] === 'second';
        const sentinel = {};
        let unread = 0;
        const unreadItem = new InvItem({get ops() { unread++; return []; }});
        Object.defineProperty(Array, 'isArray', {
            configurable: true, get() { throw sentinel; },
        });
        let calleeErrorIdentity = false;
        try { unreadItem.actions(); }
        catch (error) { calleeErrorIdentity = error === sentinel; }
        finally { Object.defineProperty(Array, 'isArray', descriptor); }
        const firstDescriptor = Object.getOwnPropertyDescriptor(Inventory, 'first');
        const name = {};
        let containsReceiver = true;
        Object.defineProperty(Inventory, 'first', {
            configurable: true,
            value(value) { containsReceiver &&= this === Inventory && value === name; return undefined; },
        });
        let containsUndefined;
        let containsNull;
        let containsZero;
        let containsErrorIdentity = false;
        try {
            containsUndefined = Inventory.contains(name);
            Inventory.first = () => null;
            containsNull = Inventory.contains(name);
            Inventory.first = () => 0;
            containsZero = Inventory.contains(name);
            Object.defineProperty(Inventory, 'first', {get() { throw sentinel; }});
            try { Inventory.contains(name); }
            catch (error) { containsErrorIdentity = error === sentinel; }
        } finally {
            Object.defineProperty(Inventory, 'first', firstDescriptor);
        }
        return {
            events, receivers, actionsIdentity, freshEmpty, retainedRow,
            calleeErrorIdentity, unread, containsReceiver,
            containsUndefined, containsNull, containsZero, containsErrorIdentity,
        };
    })()"#).unwrap();
    assert_eq!(
        result["events"],
        json!([
            "isArray.get",
            "ops.1",
            "isArray.call",
            "ops.2",
            "slice.get",
            "slice.call",
        ])
    );
    assert_eq!(result["unread"], 0);
    assert_eq!(result["containsNull"], false);
    for key in [
        "receivers",
        "actionsIdentity",
        "freshEmpty",
        "retainedRow",
        "calleeErrorIdentity",
        "containsReceiver",
        "containsUndefined",
        "containsZero",
        "containsErrorIdentity",
    ] {
        assert_eq!(result[key], true, "{key}: {result}");
    }
    iso.join();
}

#[test]
fn inventory_root_identity_and_module_only_helpers_follow_bank_page_transitions() {
    let iso = spawn();
    let identity = iso.probe("__inventoryRootIdentity").unwrap();
    let identity = identity.as_object().unwrap();
    for (name, same) in identity {
        assert_eq!(same.as_bool(), Some(true), "{name}");
    }
    let inventory = [row(Some("Apple"), 10, 3, 0, 503, (false, -1), &[])];
    let bank_side = [row(Some("Bank-side ore"), 20, 1, 0, 701, (false, -1), &[])];
    let mut snapshot = common::ingame_snapshot();
    snapshot.tick = 1;
    snapshot.inv = &inventory;
    snapshot.inv_size = 19;
    snapshot.bank_side = &bank_side;
    snapshot.bank_open = false;
    post(&iso, &snapshot, Some(700));
    assert_eq!(
        iso.probe("__inventoryRootRead()").unwrap(),
        json!({"names": ["Apple"], "capacity": 19, "inventoryCount": 3})
    );
    snapshot.tick = 2;
    snapshot.bank_open = true;
    post(&iso, &snapshot, Some(700));
    assert_eq!(
        iso.probe("__inventoryRootRead()").unwrap(),
        json!({"names": ["Bank-side ore"], "capacity": 28, "inventoryCount": 3})
    );
    snapshot.tick = 3;
    snapshot.bank_open = false;
    post(&iso, &snapshot, Some(-1));
    assert_eq!(
        iso.probe("__inventoryRootRead()").unwrap(),
        json!({"names": ["Apple"], "capacity": 19, "inventoryCount": 3})
    );
    iso.join();
}

#[test]
fn implemented_readers_preserve_posted_v8_rows_getters_and_sparse_arrays() {
    let iso = spawn();
    let result = iso.probe(r#"(() => {
        const Inventory = globalThis.__inventoryObject;
        const Equipment = globalThis.__equipmentObject;
        const host = globalThis.__rs2b0t_host;
        const snapshotDescriptor = Object.getOwnPropertyDescriptor(host, 'snapshot');
        const sizeDescriptor = Object.getOwnPropertyDescriptor(host, 'invSize');
        const usedDescriptor = Object.getOwnPropertyDescriptor(Inventory, 'used');
        const maxDescriptor = Object.getOwnPropertyDescriptor(Math, 'max');
        const countEvents = [];
        let countReads = 0;
        const counted = {
            get name() { countEvents.push('name'); return 'Apple'; },
            get count() { countEvents.push('count'); return ++countReads === 1 ? 1 : '7'; },
        };
        const countSnapshot = {
            get inv() { countEvents.push('inv'); return [null, , counted]; },
        };
        try {
            Object.defineProperty(host, 'snapshot', {
                configurable: true,
                get() { countEvents.push('snapshot'); return countSnapshot; },
            });
            const count = Inventory.count('apple');
            const retained = { name: 'Apple', count: 1, id: 10, slot: 2, ops: ['Eat'] };
            const sparse = new Array(3);
            sparse[2] = retained;
            let equipmentCountReads = 0;
            let equipmentIdReads = 0;
            const equipmentRow = {
                name: 'Sword',
                get count() { return ++equipmentCountReads === 1 ? 1 : 'two'; },
                get id() { return ++equipmentIdReads === 1 ? 2 : null; },
            };
            const current = { inv: sparse, equipment: [null, , equipmentRow] };
            Object.defineProperty(host, 'snapshot', { configurable: true, value: current, writable: true });
            const items = Inventory.items();
            retained.name = 'Pear';
            retained.ops = ['Caller action'];
            const copiedName = items[0].name;
            const freshName = Inventory.first('pear').name;
            const retainedAction = items[0].actions()[0];
            const equipment = Equipment.items();
            const capacityEvents = [];
            let capacityReads = 0;
            delete host.invSize;
            Object.defineProperty(current, 'inv_size', {
                get() { capacityEvents.push('size.' + ++capacityReads); return capacityReads === 1 ? 3 : 4; },
            });
            Object.defineProperty(host, 'snapshot', {
                configurable: true,
                get() { capacityEvents.push('snapshot'); return current; },
            });
            Object.defineProperty(Math, 'max', {
                configurable: true,
                get() {
                    capacityEvents.push('max.get');
                    return function(...args) {
                        capacityEvents.push('max.call');
                        return maxDescriptor.value.apply(this, args);
                    };
                },
            });
            Object.defineProperty(Inventory, 'used', {
                configurable: true,
                get() {
                    capacityEvents.push('used.get');
                    return function() { capacityEvents.push('used.call'); return 1; };
                },
            });
            const free = Inventory.free();
            const sentinel = {};
            Object.defineProperty(host, 'snapshot', {
                configurable: true, get() { throw sentinel; },
            });
            let snapshotErrorIdentity = false;
            try { Inventory.items(); }
            catch (error) { snapshotErrorIdentity = error === sentinel; }
            return {
                count, countEvents, itemCount: items.length, copiedName, freshName,
                retainedAction, equipment, equipmentCountReads, equipmentIdReads,
                free, capacityEvents, snapshotErrorIdentity,
            };
        } finally {
            if (snapshotDescriptor) Object.defineProperty(host, 'snapshot', snapshotDescriptor);
            else delete host.snapshot;
            if (sizeDescriptor) Object.defineProperty(host, 'invSize', sizeDescriptor);
            else delete host.invSize;
            Object.defineProperty(Inventory, 'used', usedDescriptor);
            Object.defineProperty(Math, 'max', maxDescriptor);
        }
    })()"#).unwrap();
    assert_eq!(result["count"], "07");
    assert_eq!(
        result["countEvents"],
        json!(["snapshot", "inv", "name", "name", "count", "count"])
    );
    assert_eq!(result["itemCount"], 1);
    assert_eq!(result["copiedName"], "Apple");
    assert_eq!(result["freshName"], "Pear");
    assert_eq!(result["retainedAction"], "Caller action");
    assert_eq!(
        result["equipment"],
        json!([{"name": "Sword", "count": "two", "id": null}])
    );
    assert_eq!(result["equipmentCountReads"], 2);
    assert_eq!(result["equipmentIdReads"], 2);
    assert_eq!(result["free"], 3);
    assert_eq!(
        result["capacityEvents"],
        json!([
            "snapshot",
            "size.1",
            "snapshot",
            "size.2",
            "max.get",
            "used.get",
            "used.call",
            "max.call"
        ])
    );
    assert_eq!(result["snapshotErrorIdentity"], true);
    iso.join();
}
