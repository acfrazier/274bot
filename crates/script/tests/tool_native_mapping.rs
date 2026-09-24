//! Tools exports: one native call per export (`__rs2b0t_tools`), with Rust
//! invoking the script's own callbacks.
//!
//! Expected call sequences and results are the frozen
//! `bot/api/acquisition/Tools.ts` bodies (`bestFromTiers`, `hasToolReq`,
//! `hasAllTools`, `toolRestockPlan`) over the selected-revision tool tables;
//! a `tiered` requirement stays an explicit `notImpl`.

use script::{LoadIsolate, LoadShape};
use serde_json::json;

fn probe(src_body: &str, key: &str) -> serde_json::Value {
    let src = format!(
        "import {{ bestAxe, bestPickaxe, canWieldTool, hasAllTools, hasToolReq, toolRestockPlan }} \
         from '../../api/acquisition/Tools.js';\n\
         const capture = (fn) => {{ try {{ return fn(); }} catch (e) {{ return 'THREW ' + (e && e.name) + ': ' + (e && e.message); }} }};\n\
         {src_body}\n\
         export default class T extends LoopingBot {{ loop() {{}} }}\n"
    );
    let iso = LoadIsolate::spawn(src, LoadShape::CompatClass, vec![]).unwrap();
    let value = iso.probe(key).unwrap();
    iso.join();
    value
}

#[test]
fn best_pickaxe_calls_available_in_tier_order_after_the_level_gate() {
    let actual = probe(
        r#"
const miss = [];
const hit = [];
const truthy = [];
globalThis.__out = {
    miss: bestPickaxe(30, (n) => { miss.push(n); return false; }),
    missCalls: miss,
    hit: bestPickaxe(99, (n) => { hit.push(n); return n === 'Steel pickaxe'; }),
    hitCalls: hit,
    // Frozen `level >= t.level && available(t.name)` is a truthiness test.
    truthy: bestPickaxe(99, (n) => { truthy.push(n); return n === 'Adamant pickaxe' ? 1 : 0; }),
    truthyCalls: truthy,
    // A NaN level fails every gate, so a non-function is never called.
    ineligibleNull: bestPickaxe(NaN, null),
    reachedNull: capture(() => bestPickaxe(99, null)),
};
"#,
        "__out",
    );
    assert_eq!(
        actual,
        json!({
            "miss": null,
            "missCalls": ["Mithril pickaxe", "Steel pickaxe", "Iron pickaxe", "Bronze pickaxe"],
            "hit": "Steel pickaxe",
            "hitCalls": ["Rune pickaxe", "Adamant pickaxe", "Mithril pickaxe", "Steel pickaxe"],
            "truthy": "Adamant pickaxe",
            "truthyCalls": ["Rune pickaxe", "Adamant pickaxe"],
            "ineligibleNull": null,
            "reachedNull": "THREW TypeError: available is not a function",
        })
    );
}

#[test]
fn a_throwing_callback_reaches_the_script_unchanged_and_ends_the_walk() {
    let actual = probe(
        r#"
const calls = [];
const err = new Error('cb boom');
let caught = null;
try {
    bestAxe(1, (n) => { calls.push(n); throw err; });
} catch (e) {
    caught = e;
}
globalThis.__out = { same: caught === err, message: caught && caught.message, calls };
"#,
        "__out",
    );
    assert_eq!(
        actual,
        json!({"same": true, "message": "cb boom", "calls": ["Rune axe"]})
    );
}

#[test]
fn the_level_is_converted_at_each_gated_tier() {
    let actual = probe(
        r#"
const log = [];
const level = { valueOf() { log.push('level'); return 99; } };
bestPickaxe(level, (n) => { log.push(n); return n === 'Adamant pickaxe'; });
globalThis.__out = log;
"#,
        "__out",
    );
    assert_eq!(
        actual,
        json!(["level", "Rune pickaxe", "level", "Adamant pickaxe"])
    );
}

#[test]
fn reentrant_best_axe_keeps_independent_walks() {
    let actual = probe(
        r#"
const log = [];
const hit = bestAxe(1, (n) => {
    log.push('outer:' + n);
    if (n === 'Rune axe') {
        const inner = bestAxe(1, (m) => { log.push('inner:' + m); return m === 'Mithril axe'; });
        log.push('innerHit:' + inner);
    }
    return n === 'Adamant axe';
});
globalThis.__out = { hit, log };
"#,
        "__out",
    );
    assert_eq!(
        actual,
        json!({
            "hit": "Adamant axe",
            "log": [
                "outer:Rune axe",
                "inner:Rune axe",
                "inner:Adamant axe",
                "inner:Mithril axe",
                "innerHit:Mithril axe",
                "outer:Adamant axe",
            ],
        })
    );
}

#[test]
fn has_all_tools_is_every_exact_requirement_in_order() {
    let actual = probe(
        r#"
const short = [];
const full = [];
const stock = { Tinderbox: 2, Hammer: 1 };
const boom = [];
const tiered = [];
const sparse = [];
let reads = 0;
globalThis.__out = {
    short: hasAllTools([{ name: 'Tinderbox', min: 2 }, { name: 'Hammer' }], null,
        (n) => { short.push(n); return 0; }),
    shortCalls: short,
    full: hasAllTools([{ name: 'Tinderbox', min: 2 }, { name: 'Hammer' }], null,
        (n) => { full.push(n); return stock[n] || 0; }),
    fullCalls: full,
    // `count(r.name) >= (r.min ?? 1)`: two strings compare as strings.
    stringCompare: hasAllTools([{ name: 'Tinderbox', min: '10' }], null, () => '2'),
    boom: capture(() => hasAllTools([{ name: 'A' }, { name: 'B' }], null, (n) => {
        boom.push(n);
        if (n === 'B') throw new Error('inv boom');
        return 1;
    })),
    boomCalls: boom,
    tiered: capture(() => hasAllTools([{ kind: 'tiered', skill: 'mining' }], () => 1,
        (n) => { tiered.push(n); return 1; })),
    tieredCalls: tiered,
    // `reqs.every` skips holes; an explicit undefined slot is read.
    sparse: hasAllTools([, { name: 'Tinderbox' }], null, (n) => { sparse.push(n); return 1; }),
    sparseCalls: sparse,
    undefinedSlot: capture(() => hasAllTools([undefined], null, () => 1)),
    nullReqs: capture(() => hasAllTools(null, null, () => 1)),
    nonFunction: capture(() => hasAllTools([{ name: 'Tinderbox' }], () => 1, null)),
    empty: hasAllTools([], null, null),
    nameReads: [hasAllTools([{ get name() { reads += 1; return 'Tinderbox'; } }], null, () => 1), reads],
    single: hasToolReq({ name: 'Tinderbox', min: 3 }, null, () => 3),
};
"#,
        "__out",
    );
    assert_eq!(
        actual,
        json!({
            "short": false,
            "shortCalls": ["Tinderbox"],
            "full": true,
            "fullCalls": ["Tinderbox", "Hammer"],
            "stringCompare": true,
            "boom": "THREW Error: inv boom",
            "boomCalls": ["A", "B"],
            "tiered": "THREW Error: not impl: Tools.hasAllTools",
            "tieredCalls": [],
            "sparse": true,
            "sparseCalls": ["Tinderbox"],
            "undefinedSlot": "THREW TypeError: Cannot read properties of undefined (reading 'kind')",
            "nullReqs": "THREW TypeError: Cannot read properties of null (reading 'every')",
            "nonFunction": "THREW TypeError: count is not a function",
            "empty": true,
            "nameReads": [true, 1],
            "single": true,
        })
    );
}

#[test]
fn tool_restock_plan_asks_inv_then_bank_per_exact_requirement() {
    let actual = probe(
        r#"
const log = [];
const stocked = [];
globalThis.__out = {
    plan: toolRestockPlan([{ name: 'Tinderbox', min: 2, restock: 4, equip: true }, { name: 'Hammer' }], null,
        (n) => { log.push('inv:' + n); return n === 'Hammer' ? 0 : 1; },
        (n) => { log.push('bank:' + n); return 3; }),
    log,
    stocked: toolRestockPlan([{ name: 'Tinderbox' }], null,
        (n) => { stocked.push('inv:' + n); return 5; },
        (n) => { stocked.push('bank:' + n); return 5; }),
    stockedLog: stocked,
    emptyBank: toolRestockPlan([{ name: 'Tinderbox' }], null, () => 0, () => 0),
    tiered: capture(() => toolRestockPlan([{ kind: 'tiered' }], null, () => 0, () => 1)),
    bankBoom: capture(() => toolRestockPlan([{ name: 'Tinderbox' }], null, () => 0,
        () => { throw new Error('bank boom'); })),
    nanQty: Number.isNaN(toolRestockPlan([{ name: 'Tinderbox', restock: NaN }], null, () => 0, () => 1)[0].qty),
};
"#,
        "__out",
    );
    assert_eq!(
        actual,
        json!({
            "plan": [
                {"name": "Tinderbox", "qty": 3, "equip": true},
                {"name": "Hammer", "qty": 1, "equip": false},
            ],
            "log": ["inv:Tinderbox", "bank:Tinderbox", "inv:Hammer", "bank:Hammer"],
            "stocked": [],
            "stockedLog": ["inv:Tinderbox"],
            "emptyBank": [],
            "tiered": "THREW Error: not impl: Tools.toolRestockPlan",
            "bankBoom": "THREW Error: bank boom",
            "nanQty": true,
        })
    );
}

#[test]
fn can_wield_tool_is_the_selected_attack_gate() {
    let actual = probe(
        r#"
const throwing = { valueOf() { throw new Error('unused attack'); } };
globalThis.__out = {
    spaced: canWieldTool('  Steel pickaxe  ', 5),
    stringAttack: canWieldTool('Steel axe', '5'),
    low: canWieldTool('Steel axe', 4),
    unknown: canWieldTool('Dragon pickaxe', 99),
    missing: canWieldTool(undefined, 99),
    nanAttack: canWieldTool('Steel axe', NaN),
    bronze: canWieldTool('Bronze axe', 0),
    unknownIgnoresAttack: canWieldTool('Unknown tool', throwing),
    bronzeIgnoresAttack: canWieldTool('Bronze axe', throwing),
};
"#,
        "__out",
    );
    assert_eq!(
        actual,
        json!({
            "spaced": true,
            "stringAttack": true,
            "low": false,
            "unknown": false,
            "missing": false,
            "nanAttack": false,
            "bronze": true,
            "unknownIgnoresAttack": false,
            "bronzeIgnoresAttack": true,
        })
    );
}
