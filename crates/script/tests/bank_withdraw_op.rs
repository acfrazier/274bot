//! Frozen `withdrawOp(ops, amount)` (`api/bank/bankOps.ts:5-21`): the label
//! comes off the row's own op list, for all six amounts, and a row without a
//! matching op answers `null` instead of an assumed label.

use script::{LoadIsolate, LoadShape};
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
