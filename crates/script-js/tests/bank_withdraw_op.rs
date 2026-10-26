//! Frozen `withdrawOp(ops, amount)` (`api/bank/bankOps.ts:5-21`): the label
//! comes off the row's own op list, for all six amounts, and a row without a
//! matching op answers `null` instead of an assumed label.

#[path = "common/bank_slot.rs"]
mod bank_slot;
mod common;

use script_js::{LoadIsolate, LoadShape};
use serde_json::json;

const SRC: &str = r#"
import { withdrawOp } from '../../api/bank/bankOps.js';
const hyphen = ['Withdraw-1', 'Withdraw-5', 'Withdraw-10', 'Withdraw-All', 'Withdraw-X', null, 'Examine'];
const spaced = [null, 'Withdraw 10', 'Withdraw 1'];
const all = (ops) => ['all', '10', '5', 'x', '1', 'any'].map((amount) => withdrawOp(ops, amount));
globalThis.__probe = {
    hyphen: all(hyphen),
    spaced: all(spaced),
    fifty: withdrawOp(['Withdraw-50'], '5'),
    tenIsNotOne: withdrawOp(['Withdraw-10'], '1'),
    caseFolded: withdrawOp(['WITHDRAW ALL'], 'all'),
    unknownAmount: withdrawOp(hyphen, 'fifty') === undefined,
    numberAmount: withdrawOp(hyphen, 10) === undefined,
};
export default class T extends LoopingBot { loop() {} }
"#;

#[test]
fn withdraw_op_reads_the_row_ops_for_every_frozen_amount() {
    let iso = LoadIsolate::spawn(SRC.into(), LoadShape::CompatClass, vec![]).unwrap();
    let got = iso.probe("__probe").unwrap();
    assert_eq!(
        got["hyphen"],
        json!([
            "Withdraw-All",
            "Withdraw-10",
            "Withdraw-5",
            "Withdraw-X",
            "Withdraw-1",
            "Withdraw-1"
        ])
    );
    assert_eq!(
        got["spaced"],
        json!([null, "Withdraw 10", null, null, "Withdraw 1", "Withdraw 10"]),
        "a row without All/5/X has no such op"
    );
    assert_eq!(got["fifty"], json!(null), "`5\\b` is not Withdraw-50");
    assert_eq!(got["tenIsNotOne"], json!(null), "`^…1$` is anchored");
    assert_eq!(got["caseFolded"], json!("WITHDRAW ALL"));
    assert_eq!(got["unknownAmount"], json!(true));
    assert_eq!(got["numberAmount"], json!(true));
    iso.join();
}

/// Bank API characterizations cross the real compatibility/native seat boundary.
/// Only host dispatch receipts and observed inventory are supplied.
mod native_bank_seat {
    use std::sync::Arc;

    use api::named_banks::NamedBankFacts;
    use api::quest_progress::EvidenceStamp;
    use script::native::{HostEffect, InteractionReceipt};
    use script::shim::InteractReq;
    use script_js::wire::{ItemRowInput, NativeFactsInput};
    use script_js::LoadShape;
    use serde_json::{json, Value};

    use super::bank_slot::BankSlot;
    use super::common;

    const LADDER: [&str; 4] = ["Withdraw-1", "Withdraw-5", "Withdraw-10", "Withdraw-X"];

    struct Harness {
        runner: BankSlot,
        tick: u64,
        ops: Vec<String>,
    }

    impl Harness {
        fn new(target: i32, ops: &[&str], deposit_first: bool) -> Self {
            let deposit = if deposit_first {
                "Bank.depositInventory();"
            } else {
                ""
            };
            let source = format!(
                r#"
import {{ withdrawTo }} from '../../api/thieving/stealRules.js';
import {{ Bank }} from '../../api/bank/Bank.js';
export default class T extends LoopingBot {{
    async loop() {{
        if (globalThis.__started) return;
        globalThis.__started = true;
        {deposit}
        globalThis.__gained = await withdrawTo('Lobster', {target});
    }}
}}
"#
            );
            Self::with_source(source, ops)
        }

        fn new_deposit_pair(ops: &[&str]) -> Self {
            let source = r#"
import { Bank } from '../../api/bank/Bank.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__started) return;
        globalThis.__started = true;
        globalThis.__firstSettled = false;
        globalThis.__firstRejected = false;
        globalThis.__secondSettled = false;
        globalThis.__secondRejected = false;
        globalThis.__secondValue = 'pending';
        globalThis.__secondIsVoid = false;

        Bank.depositInventory().then(
            () => { globalThis.__firstSettled = true; },
            () => {
                globalThis.__firstSettled = true;
                globalThis.__firstRejected = true;
            },
        );
        Bank.depositInventory().then(
            (value) => {
                globalThis.__secondSettled = true;
                globalThis.__secondValue = value === undefined ? null : value;
                globalThis.__secondIsVoid = value === undefined;
            },
            () => {
                globalThis.__secondSettled = true;
                globalThis.__secondRejected = true;
            },
        );
    }
}
"#;
            Self::with_source(source.to_owned(), ops)
        }

        fn with_source(source: String, ops: &[&str]) -> Self {
            Self {
                runner: BankSlot::new(
                    source,
                    LoadShape::CompatClass,
                    None,
                    Arc::new(NamedBankFacts::empty()),
                ),
                tick: 0,
                ops: ops.iter().map(|op| (*op).to_owned()).collect(),
            }
        }

        fn step(&mut self, held: i32, filler: usize) -> Vec<InteractReq> {
            self.tick += 1;
            let mut inv: Vec<_> = (0..filler)
                .map(|slot| ItemRowInput {
                    id: 526,
                    slot: i32::try_from(slot).unwrap(),
                    component_id: 3214,
                    ..ItemRowInput::nc(Some("Bones"), 1)
                })
                .collect();
            if held > 0 {
                inv.push(ItemRowInput {
                    id: 379,
                    slot: i32::try_from(filler).unwrap(),
                    component_id: 3214,
                    ..ItemRowInput::nc(Some("Lobster"), held)
                });
            }
            let bank = [ItemRowInput {
                id: 379,
                slot: 0,
                component_id: 700,
                ops: &self.ops,
                ..ItemRowInput::nc(Some("Lobster"), 100)
            }];
            let deposit_ops = vec!["Deposit-All".to_owned()];
            let side: Vec<_> = inv
                .iter()
                .map(|row| ItemRowInput {
                    component_id: 701,
                    ops: &deposit_ops,
                    ..*row
                })
                .collect();
            let input = script_js::wire::SnapshotInput {
                tick: self.tick,
                bank_open: true,
                bank_loaded: true,
                bank_generation: 1,
                inv: &inv,
                bank: &bank,
                bank_side: &side,
                ..common::ingame_snapshot()
            };
            self.runner.tick_from_input(
                self.tick,
                &input,
                NativeFactsInput {
                    bank_snapshot_generation: Some(3),
                    ..NativeFactsInput::default()
                },
            );
            self.runner
                .take_actions()
                .into_iter()
                .map(|action| {
                    let authority = action.authority();
                    self.runner.slot.complete_native_interaction(
                        &authority,
                        InteractionReceipt {
                            request_id: action.request_id.get(),
                            evidence: EvidenceStamp {
                                run: authority.run(),
                                tick: self.tick,
                                sequence: self.tick,
                            },
                            accepted: true,
                            chat_since: 0,
                        },
                    );
                    match action.effect {
                        HostEffect::Interaction(request) => request,
                        _ => panic!("withdrawal emitted a non-interaction effect"),
                    }
                })
                .collect()
        }

        fn next_click(&mut self, held: i32) -> InteractReq {
            for _ in 0..12 {
                let clicks = self.step(held, 0);
                if !clicks.is_empty() {
                    assert_eq!(clicks.len(), 1, "each click waits for its own receipt");
                    return clicks.into_iter().next().unwrap();
                }
            }
            let logs = self.runner.slot.drain_logs();
            panic!(
                "native withdrawal did not emit its next click; result={}, state={:?}, error={:?}, logs={logs:?}",
                self.result(),
                self.runner.slot.state(),
                self.runner.slot.last_error(),
            );
        }

        fn result(&self) -> Value {
            self.runner.probe("globalThis.__gained").unwrap()
        }

        fn finish(&mut self, held: i32, filler: usize, expected: i32) {
            for _ in 0..12 {
                assert!(self.step(held, filler).is_empty(), "no surplus withdraw");
                if self.result() == json!(expected) {
                    self.runner.stop();
                    return;
                }
            }
            panic!("expected gained count {expected}, got {}", self.result());
        }
    }

    #[test]
    fn a_full_pack_withdraws_nothing() {
        let mut harness = Harness::new(22, &LADDER, false);
        harness.finish(2, 27, 0);
    }

    #[test]
    fn a_withdraw_that_never_lands_ends_at_the_land_bound() {
        let mut harness = Harness::new(4, &LADDER, false);
        assert_eq!(
            harness.next_click(0),
            InteractReq::Withdraw {
                name: "Lobster".into(),
                action: "Withdraw-1".into(),
            }
        );
        for _ in 0..4 {
            assert!(harness.step(0, 0).is_empty());
        }
        assert_eq!(harness.result(), Value::Null, "inside the 2.5 second bound");
        std::thread::sleep(std::time::Duration::from_millis(2_600));
        harness.finish(0, 0, 0);
    }

    #[test]
    fn a_fixed_only_row_serves_the_withdraw_x_chunk_by_the_ladder() {
        let mut harness = Harness::new(22, &LADDER[..3], false);
        let first = harness.next_click(2);
        let InteractReq::WithdrawX {
            bank_generation, ..
        } = first
        else {
            panic!("the fixed ladder did not serve the X chunk");
        };
        assert!(bank_generation > 0);
        let ten = InteractReq::WithdrawX {
            name: "Lobster".into(),
            count: 10,
            bank_item_id: 379,
            lands_as_id: 379,
            action: "Withdraw-10".into(),
            bank_generation,
        };
        assert_eq!(first, ten);
        assert_eq!(
            harness.next_click(12),
            ten,
            "one landed click is not the 20 the chunk asked for"
        );
        harness.finish(22, 0, 20);
    }

    #[test]
    fn a_need_of_seven_clicks_five_then_one_then_one() {
        let mut harness = Harness::new(7, &LADDER, false);
        for (held, action) in [(0, "Withdraw-5"), (5, "Withdraw-1"), (6, "Withdraw-1")] {
            assert_eq!(
                harness.next_click(held),
                InteractReq::Withdraw {
                    name: "Lobster".into(),
                    action: action.into(),
                }
            );
        }
        harness.finish(7, 0, 7);
    }

    #[test]
    fn a_second_deposit_settles_null_while_the_first_is_live() {
        let mut harness = Harness::new_deposit_pair(&LADDER);
        let clicks = harness.step(0, 1);
        assert_eq!(clicks.len(), 1, "only the first deposit should click");
        assert!(
            matches!(clicks.first(), Some(InteractReq::InvButton { .. })),
            "the live first deposit should emit the inventory button: {clicks:?}"
        );
        assert_eq!(
            harness.runner.probe("__firstSettled").unwrap(),
            json!(false)
        );
        assert_eq!(
            harness.runner.probe("__firstRejected").unwrap(),
            json!(false)
        );
        assert_eq!(
            harness.runner.probe("__secondSettled").unwrap(),
            json!(true)
        );
        assert_eq!(
            harness.runner.probe("__secondRejected").unwrap(),
            json!(false)
        );
        assert_eq!(harness.runner.probe("__secondValue").unwrap(), Value::Null);
        assert_eq!(harness.runner.probe("__secondIsVoid").unwrap(), json!(true));
        assert!(
            harness.runner.slot.last_error().is_none(),
            "the second deposit resolves without a slot error"
        );
        harness.runner.stop();
    }

    #[test]
    fn withdraw_to_settles_zero_while_a_deposit_is_live() {
        let mut harness = Harness::new(7, &LADDER, true);
        let clicks = harness.step(0, 1);
        assert_eq!(clicks.len(), 1, "only the live deposit should click");
        assert!(
            matches!(clicks.first(), Some(InteractReq::InvButton { .. })),
            "the live deposit must not permit a withdrawal: {clicks:?}"
        );
        assert_eq!(
            harness.result(),
            json!(0),
            "withdrawTo settles zero on the first native-seat step while the deposit stays live"
        );
        assert!(harness.runner.slot.last_error().is_none());
        harness.runner.stop();
    }
}
