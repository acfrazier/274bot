//! Tool decisions: isolate export → native step → caller callback marshaling.
//!
//! The shim exports keep their synchronous signatures; which candidate is
//! probed, whether an answer is accepted, callback order/early exit and the
//! explicit feature errors are decided by `script::gather_tools::dispatch`.

use script::{LoadIsolate, LoadShape};

const SRC: &str = r#"
import {
    bestAxe, bestPickaxe, canWieldTool, hasAllTools, hasToolReq, toolRestockPlan,
} from '../../api/acquisition/Tools.js';

// Early exit and callback order.
const axeCalls = [];
globalThis.__axeHit = bestAxe(1, (n) => { axeCalls.push(n); return n === 'Steel axe'; });
globalThis.__axeCalls = axeCalls;

const pickCalls = [];
globalThis.__pickHit = bestPickaxe(30, (n) => { pickCalls.push(n); return false; });
globalThis.__pickCalls = pickCalls;

// Level coercions: strings, NaN, undefined (NaN), null (0) and Infinity.
globalThis.__levelString = bestPickaxe('30', (n) => n === 'Steel pickaxe');
globalThis.__levelNaN = bestPickaxe(NaN, () => true);
globalThis.__levelUndefined = bestPickaxe(undefined, () => true);
globalThis.__levelNull = bestPickaxe(null, () => true);
globalThis.__levelInfinity = bestPickaxe(Infinity, (n) => n === 'Rune pickaxe');
const nanCalls = [];
globalThis.__axeNaN = bestAxe(NaN, (n) => { nanCalls.push(n); return false; });
globalThis.__axeNaNCalls = nanCalls;

// A throwing callback propagates and ends the selection.
const boomCalls = [];
try {
    bestAxe(1, (n) => { boomCalls.push(n); throw new Error('cb boom'); });
    globalThis.__axeBoom = 'called';
} catch (e) {
    globalThis.__axeBoom = String(e && e.message ? e.message : e);
}
globalThis.__axeBoomCalls = boomCalls;

// `bestFrom` accepts only `=== true`.
const truthyCalls = [];
globalThis.__axeTruthy = bestAxe(1, (n) => {
    truthyCalls.push(n);
    return n === 'Rune axe' ? 1 : false;
});
globalThis.__axeTruthyCalls = truthyCalls;

globalThis.__wield = {
    spaced: canWieldTool('  Steel pickaxe  ', 5),
    stringAttack: canWieldTool('Steel axe', '5'),
    unknown: canWieldTool('Dragon pickaxe', 99),
    missing: canWieldTool(undefined, 99),
    nanAttack: canWieldTool('Steel axe', NaN),
    infAttack: canWieldTool('Steel axe', Infinity),
    bronze: canWieldTool('Bronze axe', 0),
};

// hasAllTools inventory branch: probe order, short circuit, thrown errors.
const shortLog = [];
globalThis.__hasAllShort = hasAllTools(
    [{ name: 'Tinderbox', min: 2 }, { name: 'Hammer' }],
    () => 1,
    (n) => { shortLog.push('inv:' + n); return 0; },
);
globalThis.__hasAllShortLog = shortLog;

const fullLog = [];
const stock = { Tinderbox: 2, Hammer: 1 };
globalThis.__hasAll = hasAllTools(
    [{ name: 'Tinderbox', min: 2 }, { name: 'Hammer' }],
    () => 1,
    (n) => { fullLog.push('inv:' + n); return stock[n] || 0; },
);
globalThis.__hasAllLog = fullLog;

const tieredLog = [];
try {
    hasAllTools([{ kind: 'tiered', skill: 'mining' }], () => 1, (n) => {
        tieredLog.push(n);
        return 1;
    });
    globalThis.__hasAllTiered = 'called';
} catch (e) {
    globalThis.__hasAllTiered = String(e && e.message ? e.message : e);
}
globalThis.__hasAllTieredLog = tieredLog;

const boomLog = [];
try {
    hasAllTools([{ name: 'A' }, { name: 'B' }], () => 1, (n) => {
        boomLog.push(n);
        if (n === 'B') throw new Error('inv boom');
        return 1;
    });
    globalThis.__hasAllBoom = 'called';
} catch (e) {
    globalThis.__hasAllBoom = String(e && e.message ? e.message : e);
}
globalThis.__hasAllBoomLog = boomLog;

// hasAllTools skill branch: invCount is not a function.
const skillLog = [];
globalThis.__hasAllSkill = hasAllTools(
    [{ name: 'Tinderbox' }],
    (n) => { skillLog.push('level:' + n); return n === 'Tinderbox' ? 42 : 0; },
    null,
);
globalThis.__hasAllSkillLog = skillLog;
globalThis.__hasAllSkillZero = hasAllTools([{ name: 'Tinderbox' }], () => 0, null);
globalThis.__hasAllSkillMissing = hasAllTools([{ name: 'Tinderbox' }], null, null);
globalThis.__hasAllSkillEmpty = hasAllTools([], null, null);

// hasToolReq: truthiness (not `=== true`), no probe without a name.
const reqLog = [];
globalThis.__hasReq = {
    truthy: hasToolReq((n) => { reqLog.push('avail:' + n); return 'yes'; }, { name: 'Tinderbox' }),
    zero: hasToolReq(() => 0, { name: 'Tinderbox' }),
    nameless: hasToolReq(() => { reqLog.push('nameless'); return true; }, {}),
    nothing: hasToolReq(() => true, null),
};
globalThis.__hasReqLog = reqLog;
try {
    hasToolReq(null, { name: 'Tinderbox' });
    globalThis.__hasReqBoom = 'called';
} catch (e) {
    globalThis.__hasReqBoom = String(e && e.message ? e.message : e);
}

// toolRestockPlan: invCount then bankCount, and no bank probe when stocked.
const planLog = [];
globalThis.__plan = toolRestockPlan(
    [{ name: 'Tinderbox', min: 2, restock: 4, equip: true }],
    () => 1,
    (n) => { planLog.push('inv:' + n); return 1; },
    (n) => { planLog.push('bank:' + n); return 3; },
);
globalThis.__planLog = planLog;

const skipLog = [];
globalThis.__planSkip = toolRestockPlan(
    [{ name: 'Tinderbox', min: 1 }],
    () => 1,
    (n) => { skipLog.push('inv:' + n); return 5; },
    (n) => { skipLog.push('bank:' + n); return 5; },
);
globalThis.__planSkipLog = skipLog;

let hammer = 'called';
try {
    toolRestockPlan([{ name: 'Hammer' }], () => 1, () => 0, () => 1);
} catch (e) {
    hammer = String(e && e.message ? e.message : e);
}
globalThis.__planHammer = hammer;

// The posted table is the decision input: reordering it reorders the probes.
globalThis.__order = (() => {
    const row = globalThis.__rs2b0t_host.content.gather_tools;
    const rows = row.axes;
    row.axes = [rows[rows.length - 1], rows[0]];
    const calls = [];
    const hit = bestAxe(1, (n) => { calls.push(n); return true; });
    row.axes = rows;
    return { hit, calls };
})();

export default class T extends LoopingBot {
    loop() {}
}
"#;

fn spawn() -> LoadIsolate {
    LoadIsolate::spawn(SRC.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

#[test]
fn exports_marshal_native_decisions_not_js_policy() {
    let iso = spawn();
    // Early exit: the second candidate is never probed.
    assert_eq!(iso.probe("__axeHit").unwrap(), "Steel axe");
    assert_eq!(
        iso.probe("__axeCalls").unwrap(),
        serde_json::json!([
            "Rune axe",
            "Adamant axe",
            "Mithril axe",
            "Black axe",
            "Steel axe"
        ])
    );
    assert_eq!(iso.probe("__pickHit").unwrap(), serde_json::Value::Null);
    assert_eq!(
        iso.probe("__pickCalls").unwrap(),
        serde_json::json!([
            "Mithril pickaxe",
            "Steel pickaxe",
            "Iron pickaxe",
            "Bronze pickaxe",
        ]),
        "mining gate skips rune/adamant before any callback"
    );

    // Level coercion stays JS `Number()` and the gate stays native.
    assert_eq!(iso.probe("__levelString").unwrap(), "Steel pickaxe");
    assert_eq!(iso.probe("__levelNaN").unwrap(), serde_json::Value::Null);
    assert_eq!(
        iso.probe("__levelUndefined").unwrap(),
        serde_json::Value::Null
    );
    assert_eq!(iso.probe("__levelNull").unwrap(), "Iron pickaxe");
    assert_eq!(iso.probe("__levelInfinity").unwrap(), "Rune pickaxe");
    assert_eq!(iso.probe("__axeNaN").unwrap(), serde_json::Value::Null);
    assert_eq!(
        iso.probe("__axeNaNCalls").unwrap(),
        serde_json::json!([
            "Rune axe",
            "Adamant axe",
            "Mithril axe",
            "Black axe",
            "Steel axe",
            "Iron axe",
            "Bronze axe"
        ]),
        "axe rows have no use gate"
    );

    // Callback exceptions and `=== true` acceptance.
    assert_eq!(iso.probe("__axeBoom").unwrap(), "cb boom");
    assert_eq!(
        iso.probe("__axeBoomCalls").unwrap(),
        serde_json::json!(["Rune axe"])
    );
    assert_eq!(iso.probe("__axeTruthy").unwrap(), serde_json::Value::Null);
    assert_eq!(
        iso.probe("__axeTruthyCalls").unwrap(),
        serde_json::json!([
            "Rune axe",
            "Adamant axe",
            "Mithril axe",
            "Black axe",
            "Steel axe",
            "Iron axe",
            "Bronze axe"
        ]),
        "a truthy non-true answer is not accepted"
    );

    // Wield policy: exact name, trim, Attack gate, bronze tutorial path.
    assert_eq!(
        iso.probe("__wield").unwrap(),
        serde_json::json!({
            "spaced": true,
            "stringAttack": true,
            "unknown": false,
            "missing": false,
            "nanAttack": false,
            "infAttack": true,
            "bronze": true,
        })
    );
    iso.join();
}

#[test]
fn has_all_and_restock_keep_callback_order_short_circuit_and_errors() {
    let iso = spawn();
    assert_eq!(iso.probe("__hasAllShort").unwrap(), false);
    assert_eq!(
        iso.probe("__hasAllShortLog").unwrap(),
        serde_json::json!(["inv:Tinderbox"]),
        "the unmet requirement stops the walk before Hammer"
    );
    assert_eq!(iso.probe("__hasAll").unwrap(), true);
    assert_eq!(
        iso.probe("__hasAllLog").unwrap(),
        serde_json::json!(["inv:Tinderbox", "inv:Hammer"])
    );
    assert_eq!(
        iso.probe("__hasAllTiered").unwrap(),
        "not impl: Tools.hasAllTools"
    );
    assert_eq!(
        iso.probe("__hasAllTieredLog").unwrap(),
        serde_json::json!([]),
        "the tiered error precedes any count probe"
    );
    assert_eq!(iso.probe("__hasAllBoom").unwrap(), "inv boom");
    assert_eq!(
        iso.probe("__hasAllBoomLog").unwrap(),
        serde_json::json!(["A", "B"])
    );

    assert_eq!(iso.probe("__hasAllSkill").unwrap(), true);
    assert_eq!(
        iso.probe("__hasAllSkillLog").unwrap(),
        serde_json::json!(["level:Tinderbox"])
    );
    assert_eq!(iso.probe("__hasAllSkillZero").unwrap(), false);
    assert_eq!(iso.probe("__hasAllSkillMissing").unwrap(), false);
    assert_eq!(
        iso.probe("__hasAllSkillEmpty").unwrap(),
        true,
        "empty requirements pass without a skill callback"
    );

    assert_eq!(
        iso.probe("__hasReq").unwrap(),
        serde_json::json!({"truthy": true, "zero": false, "nameless": false, "nothing": false})
    );
    assert_eq!(
        iso.probe("__hasReqLog").unwrap(),
        serde_json::json!(["avail:Tinderbox"]),
        "only the named requirement reaches the callback"
    );
    assert!(
        iso.probe("__hasReqBoom")
            .unwrap()
            .as_str()
            .unwrap_or("")
            .contains("not a function"),
        "a non-function available still throws"
    );

    assert_eq!(
        iso.probe("__plan").unwrap(),
        serde_json::json!([{"name": "Tinderbox", "qty": 3, "equip": true}])
    );
    assert_eq!(
        iso.probe("__planLog").unwrap(),
        serde_json::json!(["inv:Tinderbox", "bank:Tinderbox"])
    );
    assert_eq!(iso.probe("__planSkip").unwrap(), serde_json::json!([]));
    assert_eq!(
        iso.probe("__planSkipLog").unwrap(),
        serde_json::json!(["inv:Tinderbox"]),
        "a stocked pack never asks the bank"
    );
    assert_eq!(
        iso.probe("__planHammer").unwrap(),
        "not impl: Tools.toolRestockPlan"
    );
    iso.join();
}

#[test]
fn posted_rows_are_the_decision_input_not_the_compiled_table() {
    let iso = spawn();
    assert_eq!(
        iso.probe("__order").unwrap(),
        serde_json::json!({"hit": "Bronze axe", "calls": ["Bronze axe"]}),
        "the native step follows the posted order and stops at the first accepted row"
    );
    iso.join();
}

#[test]
fn native_step_wire_contract_is_probe_accept_and_none() {
    let iso = spawn();
    let facts = iso
        .probe("globalThis.__rs2b0t_host.content.gather_tools")
        .unwrap();
    let step = |payload: String| {
        iso.probe(&format!(
            "rustyscript.functions.__rs2b0t_tool_step({payload})"
        ))
        .unwrap()
    };
    let probe = step(format!(
        "{{op:'best', kind:'pickaxes', level:30, facts:{facts}, index:-1, accepted:false}}"
    ));
    assert_eq!(
        probe,
        serde_json::json!({"kind": "probe", "index": 0, "name": "Mithril pickaxe"})
    );
    let stale = step(format!(
        "{{op:'best', kind:'pickaxes', level:30, facts:{facts}, index:-1, accepted:true}}"
    ));
    assert_eq!(
        stale,
        serde_json::json!({"kind": "none"}),
        "an accepted answer with no probed candidate is not a hit"
    );
    let accepted = step(format!(
        "{{op:'best', kind:'pickaxes', level:30, facts:{facts}, index:0, accepted:true}}"
    ));
    assert_eq!(
        accepted,
        serde_json::json!({"kind": "done", "name": "Mithril pickaxe"})
    );
    let absent = step(stringify_json(&serde_json::json!({
        "op": "best",
        "kind": "pickaxes",
        "level": 99,
        "facts": serde_json::Value::Null,
        "index": -1,
        "accepted": false,
    })));
    assert_eq!(absent, serde_json::json!({"kind": "none"}));
    let unsupported = step("{op:'nope'}".to_string());
    assert_eq!(
        unsupported,
        serde_json::json!({"kind": "error", "feature": "Tools"})
    );
    iso.join();
}

/// JSON literal for embedding in a JS payload.
fn stringify_json(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}
