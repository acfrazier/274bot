// Catalog-run checks that do not require the optional rs2b0t checkout.
// Bright-card / pin-wide regressions live in LIVE harnesses and scenario cells.

#[path = "common/bank_slot.rs"]
mod bank_slot;
mod common;

#[test]
fn bank_picker_import_links_and_awaits_host_selection() {
    use api::named_banks::{NamedBank, NamedBankFacts};
    use api::quest_progress::EvidenceStamp;
    use api::snapshot::WorldTile;
    use script::bank::{BankPickReceipt, PickKind, SelectedBank};
    use script::native::HostEffect;
    use script::LoadShape;
    use std::sync::Arc;
    let banks = Arc::new(NamedBankFacts::from_banks(vec![
        NamedBank::new(
            "Air near",
            WorldTile {
                x: 3224,
                z: 3218,
                level: 0,
            },
        ),
        NamedBank::new(
            "Walk near",
            WorldTile {
                x: 3200,
                z: 3210,
                level: 0,
            },
        ),
    ]));
    let mut slot = bank_slot::BankSlot::new(
        r#"
import { nearestBankReachable } from '../../api/bank/BankLocations.js';
import { Navigator } from '../../event/webwalk/Navigator.js';
export default class Probe extends LoopingBot {
    async loop() {
        globalThis.picked = await nearestBankReachable({ x: 3222, z: 3218, level: 0 }, Navigator);
    }
}
"#,
        LoadShape::CompatClass,
        None,
        banks,
    );
    let mut snapshot = common::ingame_snapshot();
    slot.tick_from_input(1, &snapshot, Default::default());
    let mut actions = slot.take_actions();
    assert_eq!(
        actions.len(),
        1,
        "reachable picker starts one native selection"
    );
    let pick = actions.pop().unwrap();
    let HostEffect::BankPick(request) = &pick.effect else {
        panic!("reachable picker starts its native selection");
    };
    assert_eq!(
        request.from,
        WorldTile {
            x: 3222,
            z: 3218,
            level: 0,
        }
    );
    assert!(request.eligible.contains(&1));
    let authority = pick.authority();
    let request_id = pick.request_id.get();
    for (tick, id) in [(2, request_id - 1), (3, request_id)] {
        snapshot.tick = tick;
        slot.slot.complete_native_bank_pick(
            &authority,
            BankPickReceipt {
                request_id: id,
                evidence: EvidenceStamp {
                    run: authority.run(),
                    tick,
                    sequence: tick,
                },
                selected: SelectedBank {
                    bank_index: 1,
                    access_tile: WorldTile {
                        x: 3200,
                        z: 3210,
                        level: 0,
                    },
                    kind: PickKind::Reachable,
                    access: None,
                },
            },
        );
        slot.tick_from_input(tick, &snapshot, Default::default());
        let actual = slot.probe("typeof picked === 'undefined' ? null : [picked.name, picked.tile.x, picked.tile.z, picked.tile.level]").unwrap();
        if tick == 2 {
            assert_eq!(
                actual,
                serde_json::Value::Null,
                "a stale completion cannot settle the current selection"
            );
        } else {
            assert_eq!(
                actual,
                serde_json::json!(["Walk near", 3200, 3210, 0]),
                "the awaited result is the host's reachable bank, not a JS air-nearest substitute"
            );
        }
    }
    slot.stop();
}

#[test]
fn bank_v2_selection_publishes_the_typed_seat_result() {
    use api::named_banks::{NamedBank, NamedBankFacts};
    use api::quest_progress::EvidenceStamp;
    use api::snapshot::WorldTile;
    use script::bank::{BankPickReceipt, PickKind, SelectedBank};
    use script::native::HostEffect;
    use script::LoadShape;
    use std::sync::Arc;

    let banks = Arc::new(NamedBankFacts::from_banks(vec![
        NamedBank::new(
            "Air near",
            WorldTile {
                x: 3224,
                z: 3218,
                level: 0,
            },
        ),
        NamedBank::new(
            "Walk near",
            WorldTile {
                x: 3200,
                z: 3210,
                level: 0,
            },
        ),
    ]));
    let mut slot = bank_slot::BankSlot::new(
        r#"
export const apiVersion = 2;
let started = false;
export function tick(api) {
    if (started) return;
    started = true;
    const original = globalThis.__rs2b0t_machine_start;
    globalThis.__rs2b0t_machine_start = (family, args, hooks) => {
        const result = original(family, args, hooks);
        if (family === 'bank_select' && result.kind === 'running') {
            globalThis.bankRequestId = result.handle;
        }
        return result;
    };
    api.bankNearestReachable({ from: { x: 3222, z: 3218, level: 0 } })
        .then((result) => { globalThis.bankResult = result; });
}
"#,
        LoadShape::NativeTick,
        None,
        banks,
    );
    let mut snapshot = common::ingame_snapshot();
    slot.tick_from_input(1, &snapshot, Default::default());
    assert_eq!(
        slot.probe("globalThis.__rs2b0t_host.snapshot.bank_selection")
            .unwrap(),
        serde_json::Value::Null,
        "kind zero is the public null selection"
    );
    let mut actions = slot.take_actions();
    assert_eq!(actions.len(), 1, "the I-seat starts one typed bank pick");
    let pick = actions.pop().unwrap();
    let HostEffect::BankPick(_) = &pick.effect else {
        panic!("the typed selector emits a native bank pick");
    };
    let authority = pick.authority();
    let request_id = pick.request_id.get();
    slot.slot.complete_native_bank_pick(
        &authority,
        BankPickReceipt {
            request_id,
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: 2,
                sequence: 2,
            },
            selected: SelectedBank {
                bank_index: 1,
                access_tile: WorldTile {
                    x: 3200,
                    z: 3210,
                    level: 0,
                },
                kind: PickKind::Reachable,
                access: None,
            },
        },
    );
    snapshot.tick = 2;
    slot.tick_from_input(2, &snapshot, Default::default());
    let publication = slot
        .probe("globalThis.__rs2b0t_host.snapshot.bank_selection")
        .unwrap();
    let published_request_id = publication["request_id"]
        .as_f64()
        .expect("public I-seat request id is numeric");
    let settled_request_id = slot
        .probe("globalThis.bankRequestId")
        .unwrap()
        .as_f64()
        .expect("captured I-seat request id is numeric");
    assert_eq!(
        published_request_id, settled_request_id,
        "publication retains the I-seat request identity"
    );
    assert_eq!(publication["generation"], 1);
    assert_eq!(publication["kind"], "reachable");
    assert_eq!(publication["bank"]["name"], "Walk near");
    assert_eq!(
        publication["bank"]["tile"],
        serde_json::json!({"x": 3200, "z": 3210, "level": 0})
    );
    let settlement = slot.probe("globalThis.bankResult").unwrap();
    assert_eq!(settlement["ok"], true);
    assert_eq!(settlement["value"]["name"], publication["bank"]["name"]);
    slot.stop();
}

#[test]
fn bank_v2_replacement_supersedes_the_waiter_and_rejects_its_late_publication() {
    use api::named_banks::{NamedBank, NamedBankFacts};
    use api::quest_progress::EvidenceStamp;
    use api::snapshot::WorldTile;
    use script::bank::{BankPickReceipt, PickKind, SelectedBank};
    use script::native::HostEffect;
    use script::LoadShape;
    use std::sync::Arc;

    let banks = Arc::new(NamedBankFacts::from_banks(vec![
        NamedBank::new(
            "Old",
            WorldTile {
                x: 3224,
                z: 3218,
                level: 0,
            },
        ),
        NamedBank::new(
            "Current",
            WorldTile {
                x: 3200,
                z: 3210,
                level: 0,
            },
        ),
    ]));
    let mut slot = bank_slot::BankSlot::new(
        r#"
export const apiVersion = 2;
let started = 0;
export function tick(api) {
    if (started === 0) {
        globalThis.ids = [];
        globalThis.results = [];
        const original = globalThis.__rs2b0t_machine_start;
        globalThis.__rs2b0t_machine_start = (family, args, hooks) => {
            const result = original(family, args, hooks);
            if (family === 'bank_select' && result.kind === 'running') ids.push(result.handle);
            return result;
        };
    }
    if (started === 0 || (started === 1 && globalThis.replace)) {
        const index = started++;
        api.bankNearestReachable({ from: { x: 3222, z: 3218, level: 0 } })
            .then((result) => { results[index] = result; });
    }
}
"#,
        LoadShape::NativeTick,
        None,
        banks,
    );
    let mut snapshot = common::ingame_snapshot();
    slot.tick_from_input(1, &snapshot, Default::default());
    let mut actions = slot.take_actions();
    assert_eq!(actions.len(), 1);
    let old = actions.pop().unwrap();
    assert!(matches!(old.effect, HostEffect::BankPick(_)));

    slot.probe("globalThis.replace = true").unwrap();
    snapshot.tick = 2;
    slot.tick_from_input(2, &snapshot, Default::default());
    let mut actions = slot.take_actions();
    assert_eq!(
        actions.len(),
        1,
        "the replacement starts a second actual v2 request"
    );
    let current = actions.pop().unwrap();
    assert!(matches!(current.effect, HostEffect::BankPick(_)));
    assert_ne!(old.request_id, current.request_id);
    assert_eq!(
        slot.probe("globalThis.results[0]").unwrap(),
        serde_json::json!({"ok": false, "error": "superseded"}),
        "the older public waiter settles from its superseded abort"
    );

    let complete =
        |slot: &mut bank_slot::BankSlot, action: &script::native::HostAction, tick, index| {
            let authority = action.authority();
            slot.slot.complete_native_bank_pick(
                &authority,
                BankPickReceipt {
                    request_id: action.request_id.get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick,
                        sequence: tick,
                    },
                    selected: SelectedBank {
                        bank_index: index,
                        access_tile: WorldTile {
                            x: 3200,
                            z: 3210,
                            level: 0,
                        },
                        kind: PickKind::Reachable,
                        access: None,
                    },
                },
            );
        };
    complete(&mut slot, &old, 3, 0);
    snapshot.tick = 3;
    slot.tick_from_input(3, &snapshot, Default::default());
    assert_eq!(
        slot.probe("globalThis.results[1] ?? null").unwrap(),
        serde_json::Value::Null
    );
    assert_eq!(
        slot.probe("globalThis.__rs2b0t_host.snapshot.bank_selection")
            .unwrap(),
        serde_json::Value::Null,
        "a superseded completion cannot publish while the replacement is pending"
    );

    complete(&mut slot, &current, 4, 1);
    snapshot.tick = 4;
    slot.tick_from_input(4, &snapshot, Default::default());
    let publication = slot
        .probe("globalThis.__rs2b0t_host.snapshot.bank_selection")
        .unwrap();
    assert_eq!(publication["generation"], 2);
    assert_eq!(publication["kind"], "reachable");
    assert_eq!(publication["bank"]["name"], "Current");
    assert_eq!(
        slot.probe("globalThis.results[1].ok && globalThis.results[1].value.name")
            .unwrap(),
        "Current"
    );
    assert_eq!(
        publication["request_id"].as_f64(),
        slot.probe("globalThis.ids[1]").unwrap().as_f64()
    );
    complete(&mut slot, &old, 5, 0);
    snapshot.tick = 5;
    slot.tick_from_input(5, &snapshot, Default::default());
    assert_eq!(
        slot.probe("globalThis.__rs2b0t_host.snapshot.bank_selection")
            .unwrap(),
        publication
    );
    slot.stop();
    assert_eq!(slot.slot.bank_selection_input().kind, 0);
    complete(&mut slot, &current, 6, 1);
    assert_eq!(
        slot.slot.bank_selection_input().kind,
        0,
        "stop rejects late publication"
    );
}
