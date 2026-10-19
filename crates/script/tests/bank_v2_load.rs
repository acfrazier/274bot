//! The public v2 request keeps snapshot-polled completion over the native bank seat.
#[path = "common/bank_slot.rs"]
mod bank_slot;
mod common;

use std::sync::Arc;

use api::named_banks::NamedBankFacts;
use api::quest_progress::EvidenceStamp;
use bank_slot::BankSlot;
use script::isolate_fb::{ItemRowInput, NativeFactsInput, SnapshotInput};
use script::native::{HostEffect, InteractionReceipt};
use script::shim::InteractReq;
use script::LoadShape;
use serde_json::json;

fn runner(stale: bool) -> BankSlot {
    BankSlot::new(
        format!(
            r#"
export const apiVersion = 2;
export function tick(api) {{
    if (!globalThis.started) {{
        globalThis.started = true;
        globalThis.before = api.snapshot.withdraw_load_result_seq;
        api.request({{ op: 'withdraw-load', name: 'Lobster',
            bank_generation: api.snapshot.bank_generation - {} }});
    }}
    if (api.snapshot.withdraw_load_result_seq > globalThis.before) {{
        globalThis.completion = {{ seq: api.snapshot.withdraw_load_result_seq,
            result: api.snapshot.withdraw_load_result }};
    }}
}}
"#,
            u8::from(stale)
        ),
        LoadShape::NativeTick,
        None,
        Arc::new(NamedBankFacts::empty()),
    )
}

fn step(runner: &mut BankSlot, tick: u64, held: bool, generation: u64) -> Vec<InteractReq> {
    let ops = vec!["Withdraw-All".to_owned()];
    let bank = [ItemRowInput {
        id: 379,
        slot: 0,
        // A replacement component advances the fixture's real bank session too.
        component_id: 699 + i32::try_from(generation).unwrap(),
        ops: &ops,
        ..ItemRowInput::nc(Some("Lobster"), if held { 0 } else { 1 })
    }];
    let inv = [ItemRowInput {
        id: 379,
        slot: 0,
        component_id: 3214,
        ..ItemRowInput::nc(Some("Lobster"), 1)
    }];
    let input = SnapshotInput {
        tick,
        bank_open: true,
        bank_loaded: true,
        bank_generation: generation,
        inv: if held { &inv } else { &[] },
        bank: &bank,
        ..common::ingame_snapshot()
    };
    runner.tick_from_input(
        tick,
        &input,
        NativeFactsInput {
            bank_snapshot_generation: Some(if held { 4 } else { 3 }),
            ..NativeFactsInput::default()
        },
    );
    assert!(
        runner.slot.last_error().is_none(),
        "v2 load must settle without a slot error: {:?}",
        runner.slot.last_error()
    );
    runner
        .take_actions()
        .into_iter()
        .map(|action| {
            let authority = action.authority();
            runner.slot.complete_native_interaction(
                &authority,
                InteractionReceipt {
                    request_id: action.request_id.get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick,
                        sequence: tick,
                    },
                    accepted: true,
                    chat_since: 0,
                },
            );
            let HostEffect::Interaction(request) = action.effect else {
                panic!("load must use native interaction effects");
            };
            request
        })
        .collect()
}

#[test]
fn public_v2_withdraw_load_publishes_only_the_seat_completion() {
    let mut runner = runner(false);
    assert_eq!(
        step(&mut runner, 1, false, 1),
        vec![InteractReq::InvButton {
            id: 379,
            slot: 0,
            component: 700,
            operation: 1,
            bank_generation: 1,
        }]
    );
    assert_eq!(runner.slot.withdraw_load_result(), (0, false));
    assert!(step(&mut runner, 2, false, 1).is_empty());
    assert_eq!(
        runner.slot.withdraw_load_result(),
        (0, false),
        "a dispatch receipt is not inventory settlement"
    );
    assert!(step(&mut runner, 3, true, 1).is_empty());
    assert_eq!(runner.slot.withdraw_load_result(), (1, true));
    assert!(step(&mut runner, 4, true, 1).is_empty());
    assert_eq!(
        runner.probe("globalThis.completion").unwrap(),
        json!({"seq": 1, "result": true})
    );
    assert_eq!(
        runner.slot.withdraw_load_result(),
        (1, true),
        "unchanged polls do not republish a result"
    );
}

#[test]
fn public_v2_withdraw_load_rejects_a_stale_caller_generation_without_clicking() {
    let mut runner = runner(true);
    assert!(step(&mut runner, 1, false, 1).is_empty());
    assert_eq!(runner.slot.withdraw_load_result(), (1, false));
    assert!(step(&mut runner, 2, false, 1).is_empty());
    assert_eq!(
        runner.probe("globalThis.completion").unwrap(),
        json!({"seq": 1, "result": false})
    );
}

#[test]
fn public_v2_withdraw_load_rejects_a_generation_change_while_waiting() {
    let mut runner = runner(false);
    assert_eq!(step(&mut runner, 1, false, 1).len(), 1);
    assert_eq!(runner.slot.withdraw_load_result(), (0, false));
    assert!(step(&mut runner, 2, true, 2).is_empty());
    assert_eq!(
        runner.slot.withdraw_load_result(),
        (1, false),
        "a new session is stale even if it has more inventory"
    );
    assert!(step(&mut runner, 3, true, 2).is_empty());
    assert_eq!(
        runner.probe("globalThis.completion").unwrap(),
        json!({"seq": 1, "result": false})
    );
}
