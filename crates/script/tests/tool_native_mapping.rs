//! Tools exports: one native call per export (`__rs2b0t_tools`), with Rust
//! invoking the script's own callbacks.
//!
//! Expected call sequences and results preserve frozen `Tools.ts` callback
//! semantics while tiered requirements use the selected native tool gates.

use script::{LoadIsolate, LoadShape};
use serde_json::json;

fn probe(src_body: &str, key: &str) -> serde_json::Value {
    let src = format!(
        "import {{ AXES, PICKAXES, axeReq, bestAxe, bestPickaxe, bankHasBetterGatherTool, canWieldTool, \
         hasAllTools, hasToolReq, missingToolLabels, pickaxeReq, toolKeepNames, toolKitLabel, toolRestockPlan }} \
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
fn has_all_tools_is_every_exact_and_tiered_requirement_in_order() {
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
    tiered: hasAllTools([{
        kind: 'tiered', skill: 'mining', tiers: [{ name: 'Rune pickaxe', level: 41 }, { name: 'Steel pickaxe', level: 6 }]
    }], (s) => { tiered.push('skill:' + s); return 30; },
        (n) => { tiered.push(n); return n === 'Steel pickaxe'; }),
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
            "tiered": true,
            "tieredCalls": ["skill:mining", "Steel pickaxe"],
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
fn tool_restock_plan_preserves_exact_and_tiered_callback_order() {
    let actual = probe(
        r#"
const log = [];
const stocked = [];
const tieredLog = [];
const tieredReq = {
    kind: 'tiered',
    skill: 'mining',
    label: 'pickaxe',
    equip: true,
    tiers: [
        { name: 'Rune pickaxe', level: 41 },
        { name: 'Adamant pickaxe', level: 31 },
        { name: 'Mithril pickaxe', level: 21 },
        { name: 'Steel pickaxe', level: 6 },
        { name: 'Bronze pickaxe', level: 0 },
    ],
};
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
    tiered: toolRestockPlan([tieredReq], (s) => { tieredLog.push('skill:' + s); return 30; },
        (n) => { tieredLog.push('inv:' + n); return 0; },
        (n) => { tieredLog.push('bank:' + n); return n === 'Steel pickaxe' ? 1 : 0; }),
    tieredLog,
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
            "tiered": [{"name": "Steel pickaxe", "qty": 1, "equip": true}],
            "tieredLog": [
                "skill:mining",
                "inv:Mithril pickaxe", "bank:Mithril pickaxe",
                "inv:Steel pickaxe", "bank:Steel pickaxe",
                "inv:Steel pickaxe", "bank:Steel pickaxe",
            ],
            "bankBoom": "THREW Error: bank boom",
            "nanQty": true,
        })
    );
}

#[test]
fn tiered_helpers_use_native_tools_and_preserve_helper_results() {
    let actual = probe(
        r#"
const tiers = [
    { name: 'Rune pickaxe', level: 41 },
    { name: 'Steel pickaxe', level: 6 },
    { name: 'Bronze pickaxe', level: 0 },
];
const req = { kind: 'tiered', skill: 'mining', tiers, label: 'pickaxe', equip: true };
const bankReq = { ...req };
const bankCalls = [];
const axe = axeReq(false);
const pickaxe = pickaxeReq();
globalThis.__out = {
    keep: toolKeepNames([{ name: 'Hammer' }, req, { name: 'Steel pickaxe' }]),
    missing: missingToolLabels([req, { name: 'Hammer' }], () => 30, () => 0),
    label: toolKitLabel([req, { name: 'Hammer' }], () => 30, (n) => n === 'Steel pickaxe' ? 1 : 0),
    fallbackLabel: toolKitLabel([req, { name: 'Hammer' }], () => 30, () => 0),
    emptyLabel: toolKitLabel([], null, null),
    axeShape: [axe.kind, axe.skill, axe.label, axe.equip, axe.tiers === AXES],
    pickaxeShape: [pickaxe.kind, pickaxe.skill, pickaxe.label, pickaxe.equip, pickaxe.tiers === PICKAXES],
    bankBetter: bankHasBetterGatherTool([bankReq], (s) => { bankCalls.push('skill:' + s); return 30; },
        (n) => { bankCalls.push('inv:' + n); return 0; },
        (n) => { bankCalls.push('bank:' + n); return n === 'Steel pickaxe' ? 1 : 0; }),
    bankCalls,
    bankAlreadyBest: bankHasBetterGatherTool([bankReq], () => 30,
        (n) => n === 'Steel pickaxe' ? 1 : 0, () => 1),
    exactBankNotTool: bankHasBetterGatherTool([{ name: 'Hammer' }], null, () => 0, () => 1),
};
"#,
        "__out",
    );
    assert_eq!(
        actual,
        json!({
            "keep": ["Hammer", "Rune pickaxe", "Steel pickaxe", "Bronze pickaxe"],
            "missing": ["pickaxe", "Hammer"],
            "label": "Steel pickaxe + Hammer",
            "fallbackLabel": "pickaxe (bronze→rune) + Hammer",
            "emptyLabel": "gear",
            "axeShape": ["tiered", "woodcutting", "axe", false, true],
            "pickaxeShape": ["tiered", "mining", "pickaxe", true, true],
            "bankBetter": true,
            "bankCalls": [
                "skill:mining",
                "inv:Steel pickaxe", "bank:Steel pickaxe",
                "inv:Steel pickaxe", "bank:Steel pickaxe",
            ],
            "bankAlreadyBest": false,
            "exactBankNotTool": false,
        })
    );
}

#[test]
fn tool_tiers_are_projected_from_the_posted_native_content() {
    let actual = probe(
        r#"
const tools = globalThis.__rs2b0t_host.content.gather_tools;
tools.axes = [{ name: 'Black axe', use_level: null, wield_attack: 10 }];
tools.pickaxes = [
    { name: 'Steel pickaxe', use_level: 6, wield_attack: 5 },
    { name: 'Iron pickaxe', use_level: 0, wield_attack: 1 },
];
const axes = globalThis.__rs2b0t_tools('toolTiers', 'axes');
const pickaxes = globalThis.__rs2b0t_tools('toolTiers', 'pickaxes');
globalThis.__out = {
    axes,
    pickaxes,
    woodcuttingZero: hasToolReq({ kind: 'tiered', skill: 'woodcutting', tiers: axes }, () => 0, (n) => n === 'Black axe' ? 1 : 0),
    miningFive: hasToolReq({ kind: 'tiered', skill: 'mining', tiers: pickaxes }, () => 5, (n) => n === 'Steel pickaxe' ? 1 : 0),
    miningSix: hasToolReq({ kind: 'tiered', skill: 'mining', tiers: pickaxes }, () => 6, (n) => n === 'Steel pickaxe' ? 1 : 0),
};
"#,
        "__out",
    );
    assert_eq!(
        actual,
        json!({
            "axes": [{"name": "Black axe", "level": 0, "attackLevel": 10}],
            "pickaxes": [
                {"name": "Steel pickaxe", "level": 6, "attackLevel": 5},
                {"name": "Iron pickaxe", "level": 0, "attackLevel": 1},
            ],
            "woodcuttingZero": true,
            "miningFive": false,
            "miningSix": true,
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

/// Relational operators are JS's own: `ToPrimitive` on objects (`valueOf`
/// before `toString`, arrays, `String` wrappers), BigInt, and string
/// comparison by UTF-16 code units. Each case compares the export with the
/// frozen `count(req.name) >= (req.min ?? 1)` evaluated inline.
#[test]
fn comparisons_follow_js_to_primitive_bigint_and_code_units() {
    let actual = probe(
        r#"
const cases = [
    [{ valueOf() { return '5'; } }, '10'],
    [[5], '10'],
    [{}, '10'],
    [new String('5'), '10'],
    [5n, 1],
    ['\uDC00', '\uFFFD'],
    [{ [Symbol.toPrimitive]: (hint) => (hint === 'number' ? 1 : '9') }, 2],
];
globalThis.__out = cases.map(([have, min]) => {
    const native = capture(() => hasToolReq({ name: 'x', min }, null, () => have));
    const frozen = capture(() => have >= (min ?? 1));
    return { native, frozen, same: native === frozen };
});
"#,
        "__out",
    );
    let rows = actual.as_array().expect("rows");
    assert_eq!(rows.len(), 7);
    for row in rows {
        assert_eq!(row["same"], true, "{row}");
    }
    assert_eq!(rows[0]["native"], true, "'5' >= '10' compares strings");
    assert_eq!(rows[4]["native"], true, "5n >= 1 is a BigInt comparison");
    assert_eq!(
        rows[5]["native"], false,
        "a lone surrogate compares by UTF-16 code unit, below U+FFFD"
    );
}

/// A `hasAllTools` walk over 2^27 holes runs no JS at all; the watchdog's
/// termination must still stop it within the tick budget, and the isolate
/// must stay usable. The same holds for a `for...of` whose callback is a
/// builtin (`chooseTarget(sparse, Boolean)`).
#[test]
fn a_native_loop_over_holes_is_interrupted_by_the_tick_budget() {
    for body in [
        "globalThis.__rs2b0t_tools('hasAllTools', a, null, () => 1)",
        "globalThis.__rs2b0t_choose_target(a, Boolean)",
    ] {
        let src = format!(
            "export function tick(api) {{\n\
                 if (!globalThis.__started) {{\n\
                     globalThis.__started = true;\n\
                     return;\n\
                 }}\n\
                 globalThis.__entered = (globalThis.__entered | 0) + 1;\n\
                 globalThis.__rs_n = (globalThis.__rs_n || 0) + 1;\n\
                 if (globalThis.__entered === 1) {{\n\
                     const a = []; a.length = 2 ** 27;\n\
                     try {{ for (;;) {{ {body}; }} }} finally {{ globalThis.__finally = true; }}\n\
                     globalThis.__after = true;\n\
                 }}\n\
             }}"
        );
        let iso = LoadIsolate::spawn(src, LoadShape::NativeTick, vec![]).unwrap();
        // `spawn` returns before V8 setup; time the loop from Ready, not setup.
        let setup = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while matches!(iso.poll_ready(), script::Ready::Pending) {
            assert!(
                std::time::Instant::now() < setup,
                "isolate setup did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        iso.on_game_tick(1);
        assert_eq!(
            iso.probe("globalThis.__started").unwrap(),
            true,
            "{body}: first tick must publish the slow-loop start handshake"
        );
        let mut logs = Vec::new();
        iso.resume();
        iso.on_game_tick(2);
        let trigger_at = std::time::Instant::now() + std::time::Duration::from_millis(60);
        while std::time::Instant::now() < trigger_at {
            std::thread::yield_now();
        }
        iso.pause();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            logs.extend(iso.drain_logs());
            if logs.iter().any(|line| line.contains("slow tick")) {
                break;
            }
            std::thread::yield_now();
        }
        assert!(
            logs.iter().any(|line| line.contains("slow tick")),
            "{body}: the proving native loop must be interrupted: {logs:?}"
        );
        iso.resume();
        assert_eq!(
            iso.probe("globalThis.__entered").unwrap(),
            1,
            "{body}: the proving tick entered exactly once"
        );
        iso.on_game_tick(3);
        let n = iso
            .probe("__rs_n")
            .expect("isolate must stay usable after an interrupted native loop")
            .as_i64()
            .unwrap_or(0);
        assert_eq!(n, 2, "{body}: the next tick ran exactly once, n={n}");
        assert_eq!(
            iso.probe("globalThis.__after === undefined && globalThis.__finally === undefined")
                .unwrap(),
            true,
            "{body}: no script code runs after a terminated call"
        );
        iso.join();
    }
}
