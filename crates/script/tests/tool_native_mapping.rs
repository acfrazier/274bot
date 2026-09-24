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
    let step = |payload: &str| {
        iso.probe(&format!(
            "rustyscript.functions.__rs2b0t_tool_step({payload})"
        ))
        .unwrap()
    };
    assert_eq!(
        step("{op:'best', index:-1, accepted:false, has_next:true}"),
        serde_json::json!({"kind": "need_row", "index": 0})
    );
    assert_eq!(
        step("{op:'best', index:-1, accepted:false, has_next:true, row_index:0, row:{name:'Mithril pickaxe', use_skill:'mining'}}"),
        serde_json::json!({"kind": "need_level", "index": 0})
    );
    assert_eq!(
        step("{op:'best', index:-1, accepted:false, has_next:true, row_index:0, row:{name:'Mithril pickaxe', use_skill:'mining'}, level_ok:true}"),
        serde_json::json!({"kind": "probe", "index": 0, "name": "Mithril pickaxe"})
    );
    assert_eq!(
        step("{op:'best', index:-1, accepted:true, has_next:true}"),
        serde_json::json!({"kind": "none"}),
        "an accepted answer with no probed candidate is not a hit"
    );
    assert_eq!(
        step("{op:'best', index:0, accepted:true, row_index:0, row:{name:'Mithril pickaxe', use_skill:'mining'}}"),
        serde_json::json!({"kind": "done", "name": "Mithril pickaxe"})
    );
    assert_eq!(
        step("{op:'best', index:-1, accepted:false, has_next:false}"),
        serde_json::json!({"kind": "none"})
    );
    assert_eq!(
        step("{op:'nope'}"),
        serde_json::json!({"kind": "error", "feature": "Tools"})
    );
    iso.join();
}

#[test]
fn coercion_and_lazy_access_preserve_existing_export_behavior() {
    let src = r#"
import { bestAxe, bestPickaxe, canWieldTool, hasAllTools, toolRestockPlan } from '../../api/acquisition/Tools.js';
const results = {};
function capture(name, fn) {
    try { results[name] = fn(); } catch (e) { results[name] = 'THREW: ' + e.message; }
}
const throwing = { valueOf() { throw new Error('unused numeric input'); } };
capture('axeIgnoresLevel', () => bestAxe(throwing, () => true));
capture('unknownIgnoresAttack', () => canWieldTool('Unknown tool', throwing));
capture('bronzeIgnoresAttack', () => canWieldTool('Bronze axe', throwing));
capture('levelConversions', () => {
    const log = [];
    const level = { valueOf() { log.push('level'); return 99; } };
    bestPickaxe(level, name => { log.push(name); return name === 'Adamant pickaxe'; });
    return log;
});
capture('stringComparison', () => hasAllTools([{ name: 'Tinderbox', min: '10' }], null, () => '2'));
capture('hexMinimum', () => hasAllTools([{ name: 'Tinderbox', min: '0x10' }], null, () => 16));
capture('shortCircuitGetter', () => {
    let reads = 0;
    const result = hasAllTools([
        { name: 'Tinderbox', min: 2 },
        { get name() { reads++; return 'Hammer'; } },
    ], null, () => 0);
    return [result, reads];
});
capture('nanQuantity', () => Number.isNaN(toolRestockPlan([{name:'Tinderbox', restock:NaN}], null, () => 0, () => 1)[0].qty));
capture('infiniteQuantity', () => toolRestockPlan([{name:'Tinderbox', restock:Infinity}], null, () => 0, () => Infinity)[0].qty === Infinity);
globalThis.__coercionResults = results;
export default class T extends LoopingBot { loop() {} }
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let actual = iso.probe("__coercionResults").unwrap();
    iso.join();
    assert_eq!(
        actual,
        serde_json::json!({
            "axeIgnoresLevel": "Rune axe",
            "unknownIgnoresAttack": false,
            "bronzeIgnoresAttack": true,
            "levelConversions": ["level", "Rune pickaxe", "level", "Adamant pickaxe"],
            "stringComparison": true,
            "hexMinimum": true,
            "shortCircuitGetter": [false, 0],
            "nanQuantity": true,
            "infiniteQuantity": true,
        })
    );
}

#[test]
fn skipped_throwing_getter_does_not_abort_the_host() {
    let src = r#"
import { hasAllTools } from '../../api/acquisition/Tools.js';
let threw = 'no';
let result;
try {
    result = hasAllTools([
        { name: 'Tinderbox', min: 2 },
        { get name() { throw new Error('skipped getter'); } },
    ], null, () => 0);
} catch (e) {
    threw = String(e && e.message ? e.message : e);
}
globalThis.__throwingGetter = { result, threw };
export default class T extends LoopingBot { loop() {} }
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let actual = iso.probe("__throwingGetter").unwrap();
    iso.join();
    assert_eq!(
        actual,
        serde_json::json!({"result": false, "threw": "no"}),
        "a skipped requirement getter must not be serialized through serde_v8"
    );
}

#[test]
fn reentrant_best_axe_keeps_independent_candidate_cursors() {
    let src = r#"
import { bestAxe } from '../../api/acquisition/Tools.js';
const log = [];
const hit = bestAxe(1, (n) => {
    log.push('outer:' + n);
    if (n === 'Rune axe') {
        const inner = bestAxe(1, (m) => {
            log.push('inner:' + m);
            return m === 'Bronze axe';
        });
        log.push('innerHit:' + inner);
    }
    return n === 'Steel axe';
});
globalThis.__reenter = { hit, log };
export default class T extends LoopingBot { loop() {} }
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let actual = iso.probe("__reenter").unwrap();
    iso.join();
    assert_eq!(
        actual,
        serde_json::json!({
            "hit": "Steel axe",
            "log": [
                "outer:Rune axe",
                "inner:Rune axe",
                "inner:Adamant axe",
                "inner:Mithril axe",
                "inner:Black axe",
                "inner:Steel axe",
                "inner:Iron axe",
                "inner:Bronze axe",
                "innerHit:Bronze axe",
                "outer:Adamant axe",
                "outer:Mithril axe",
                "outer:Black axe",
                "outer:Steel axe",
            ],
        })
    );
}

#[test]
fn has_all_preserves_every_holes_length_and_name_reads() {
    let src = r#"
import { hasAllTools } from '../../api/acquisition/Tools.js';
const sparseCalls = [];
const sparse = hasAllTools([, {name:'Tinderbox'}], null, name => { sparseCalls.push(name); return 1; });
const rows = [{name:'Tinderbox'}];
const appendCalls = [];
const appended = hasAllTools(rows, null, name => {
    appendCalls.push(name);
    if (name === 'Tinderbox') rows.push({name:'Hammer'});
    return name === 'Tinderbox' ? 1 : 0;
});
let reads = 0;
const getterCalls = [];
const named = hasAllTools([{ get name() { return ++reads === 1 ? 'Tinderbox' : 'Hammer'; } }], null, name => {
    getterCalls.push(name); return 1;
});
globalThis.__everyResult = {sparse, sparseCalls, appended, appendCalls, named, reads, getterCalls};
export default class T extends LoopingBot { loop() {} }
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let actual = iso.probe("__everyResult").unwrap();
    iso.join();
    assert_eq!(
        actual,
        serde_json::json!({
            "sparse": true, "sparseCalls": ["Tinderbox"],
            "appended": true, "appendCalls": ["Tinderbox"],
            "named": true, "reads": 2, "getterCalls": ["Hammer"],
        })
    );
}

#[test]
fn has_all_and_has_req_preserve_explicit_slots_and_second_name_reads() {
    let src = r#"
import { hasAllTools, hasToolReq } from '../../api/acquisition/Tools.js';
const undefCalls = [];
const explicitUndef = hasAllTools([undefined, {name:'Tinderbox'}], null, name => { undefCalls.push(name); return 1; });
const nullCalls = [];
const explicitNull = hasAllTools([null, {name:'Tinderbox'}], null, name => { nullCalls.push(name); return 1; });
const holeOnlyCalls = [];
const holeOnly = hasAllTools(Array(2), null, name => { holeOnlyCalls.push(name); return 0; });
const rows = [{name:'Tinderbox'}, {name:'Hammer'}];
const deleteCalls = [];
const deleted = hasAllTools(rows, null, name => {
    deleteCalls.push(name);
    if (name === 'Tinderbox') delete rows[1];
    return 1;
});
let skillReads = 0;
const skillCalls = [];
const skillNamed = hasAllTools([{ get name() { return ++skillReads === 1 ? 'Tinderbox' : 'Hammer'; } }], (name) => { skillCalls.push(name); return 1; }, null);
let reqReads = 0;
const reqCalls = [];
const reqNamed = hasToolReq(name => { reqCalls.push(name); return true; }, { get name() { return ++reqReads === 1 ? 'Tinderbox' : 'Hammer'; } });
globalThis.__slotResult = {
    explicitUndef, undefCalls, explicitNull, nullCalls,
    holeOnly, holeOnlyCalls, deleted, deleteCalls,
    skillNamed, skillReads, skillCalls, reqNamed, reqReads, reqCalls,
};
export default class T extends LoopingBot { loop() {} }
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let actual = iso.probe("__slotResult").unwrap();
    iso.join();
    assert_eq!(
        actual,
        serde_json::json!({
            "explicitUndef": false, "undefCalls": [],
            "explicitNull": false, "nullCalls": [],
            "holeOnly": true, "holeOnlyCalls": [],
            "deleted": true, "deleteCalls": ["Tinderbox"],
            "skillNamed": true, "skillReads": 2, "skillCalls": ["Hammer"],
            "reqNamed": true, "reqReads": 2, "reqCalls": ["Hammer"],
        })
    );
}
