//! Boost potions: isolate export -> native step -> caller callback marshaling.
//!
//! The shim exports keep their synchronous signatures; the descriptor rows, the
//! planning defaults, the boost floor, the levels-before-held callback order,
//! the first-match exit and the returned-object identity are decided by
//! `script::boost_potions::dispatch`.

use script::{LoadIsolate, LoadShape};

const SRC: &str = r#"
import {
    BOOST_FLOOR, BOOST_POTIONS, EMPTY_VIAL, SUPER_ATTACK, SUPER_STRENGTH,
    boostFaded, plannedPotions, potionToSip,
} from '../../api/combat/boostPotions.js';

const capture = (fn) => {
    try {
        fn();
        return 'called';
    } catch (e) {
        return String(e && e.message ? e.message : e);
    }
};

// --- the exported descriptors ------------------------------------------------
globalThis.__exports = {
    floor: BOOST_FLOOR,
    vial: EMPTY_VIAL,
    order: BOOST_POTIONS.map(potion => potion.short),
    flasks: BOOST_POTIONS.map(potion => potion.flask),
    skills: BOOST_POTIONS.map(potion => potion.skill),
    attackIdentity: BOOST_POTIONS[0] === SUPER_ATTACK,
    strengthIdentity: BOOST_POTIONS[1] === SUPER_STRENGTH,
    attackDoses: SUPER_ATTACK.doses,
    strengthDoses: SUPER_STRENGTH.doses,
};

// --- boostFaded ---------------------------------------------------------------
globalThis.__faded = {
    unboosted: boostFaded(70, 70),
    onThreshold: boostFaded(70, 77),
    aboveThreshold: boostFaded(70, 78),
    fresh: boostFaded(70, 85),
    levelOne: [boostFaded(1, 1), boostFaded(1, 2)],
    unread: boostFaded(0, 0),
    drained: boostFaded(70, 60),
    defaultFloor: boostFaded(70, 77, undefined),
    explicitFloor: [boostFaded(70, 77, 0.2), boostFaded(70, 77, 0.05), boostFaded(70, 77, null)],
    strings: boostFaded('70', '77'),
    nonFinite: [boostFaded(70, Infinity), boostFaded(70, NaN), boostFaded(NaN, NaN)],
};

// --- plannedPotions -----------------------------------------------------------
const facts = (carry) => plannedPotions(carry).map(plan => ({
    flask: plan.flask,
    want: plan.want,
    short: plan.potion.short,
}));

globalThis.__plans = {
    defaults: facts([]),
    named: facts([{ item: 'Super attack(4)', qty: 3 }]),
    trimmedCase: facts([{ item: '  super ATTACK(2)  ', qty: 5 }]),
    unrelated: facts([{ item: 'Lobster', qty: 20 }, { item: 'Prayer potion(4)', qty: 2 }]),
    firstMatch: facts([{ item: 'Super attack(1)', qty: 7 }, { item: 'Super attack(4)', qty: 2 }]),
    bothNamed: facts([{ item: 'Super attack(4)', qty: 2 }, { item: 'Super strength(2)', qty: 4 }]),
    order: facts([{ item: 'Super strength(4)', qty: 1 }]).map(plan => plan.flask),
    countPassthrough: facts([{ item: 'Super strength(2)', qty: 'lots' }])[1].want,
};

// The plan keeps the exported descriptor identity, never a rebuilt row.
globalThis.__planIdentity = (() => {
    const defaults = plannedPotions([]);
    const named = plannedPotions([{ item: 'Super strength(4)', qty: 2 }]);
    return {
        attack: defaults[0].potion === SUPER_ATTACK,
        strength: defaults[1].potion === SUPER_STRENGTH,
        namedIsTheDescriptor: named[1].potion === SUPER_STRENGTH,
        dosesAreTheExportedOnes: named[1].potion.doses === SUPER_STRENGTH.doses,
    };
})();

// Each potion's scan stops at its own first match, so a later entry is never read.
globalThis.__planEarlyExit = (() => {
    const carry = [
        { item: 'Super attack(4)', qty: 1 },
        { item: 'Super strength(2)', qty: 3 },
        { get item() { throw new Error('third entry read'); } },
    ];
    try {
        return facts(carry);
    } catch (e) {
        return String(e && e.message ? e.message : e);
    }
})();

// A scan that has not matched keeps reading the caller's own entries.
globalThis.__planIterator = (() => {
    const carry = [{ item: 'Lobster', qty: 1 }, { get item() { throw new Error('entry two'); } }];
    return capture(() => plannedPotions(carry));
})();

// The carried item is read once per dose comparison, as the frozen `find` does.
globalThis.__planReads = (() => {
    let reads = 0;
    const carry = [{ get item() { reads += 1; return 'Super attack(2)'; }, qty: 1 }];
    try {
        plannedPotions(carry);
    } catch (e) {
        return String(e && e.message ? e.message : e);
    }
    return reads;
})();

globalThis.__planErrors = {
    notIterable: capture(() => plannedPotions(undefined)),
    nullEntry: capture(() => plannedPotions([null])),
    nonStringItem: capture(() => plannedPotions([{ item: 5, qty: 1 }])),
};

// --- potionToSip --------------------------------------------------------------
const plan = (potion, over = {}) => ({ potion, flask: potion.flask, want: 1, ...over });

// The plan whose boost has faded wins; later plans are never probed.
globalThis.__sip = (() => {
    const log = [];
    const attack = plan(SUPER_ATTACK);
    const strength = plan(SUPER_STRENGTH);
    const chosen = potionToSip({
        plans: [attack, strength],
        held: (p) => { log.push('held:' + p.potion.short); return 1; },
        levels: (skill) => {
            log.push('levels:' + skill);
            return skill === 'strength' ? { base: 70, effective: 70 } : { base: 70, effective: 85 };
        },
    });
    return { chosenIsStrength: chosen === strength, log };
})();

// Attack wins a tick both could use, and every reached plan asks levels first.
globalThis.__sipOrder = (() => {
    const log = [];
    const attack = plan(SUPER_ATTACK);
    const strength = plan(SUPER_STRENGTH);
    const chosen = potionToSip({
        plans: [attack, strength],
        held: (p) => { log.push('held:' + p.potion.short); return 1; },
        levels: (skill) => { log.push('levels:' + skill); return { base: 70, effective: 70 }; },
    });
    return { chosenIsAttack: chosen === attack, log };
})();

// An empty pack still asks that plan's levels first, then moves on.
globalThis.__sipHeld = (() => {
    const log = [];
    const attack = plan(SUPER_ATTACK);
    const strength = plan(SUPER_STRENGTH);
    const chosen = potionToSip({
        plans: [attack, strength],
        held: (p) => { log.push('held:' + p.potion.short); return p === attack ? 0 : 2; },
        levels: (skill) => { log.push('levels:' + skill); return { base: 70, effective: 70 }; },
    });
    return { chosenIsStrength: chosen === strength, log };
})();

// Every no-sip path: a held boost, an empty pack, a drained skill, no plans.
globalThis.__sipNone = (() => {
    const levels = [];
    const attack = plan(SUPER_ATTACK);
    const probe = (base, effective, held) => potionToSip({
        plans: [attack],
        held: () => held,
        levels: () => { levels.push('levels'); return { base, effective }; },
    });
    const emptyPlans = potionToSip({
        plans: [],
        held: () => 9,
        levels: () => { levels.push('empty-plans'); return { base: 70, effective: 70 }; },
    });
    return {
        heldBoost: probe(70, 85, 1),
        emptyPack: probe(70, 70, 0),
        drained: probe(70, 60, 1),
        emptyPlans,
        levels,
    };
})();

// The caller's own plan objects decide the skill as well as the result.
globalThis.__sipCaller = (() => {
    let skillReads = 0;
    const seen = [];
    const custom = {
        potion: { short: 'Rng', flask: 'Super ranged(3)', doses: [], get skill() { skillReads += 1; return 'ranged'; } },
        flask: 'Super ranged(4)',
        want: 2,
    };
    const chosen = potionToSip({
        plans: [custom],
        held: () => { seen.push('held'); return 1; },
        levels: (name) => { seen.push('levels:' + name); return { base: 40, effective: 40 }; },
    });
    return { sameObject: chosen === custom, skillReads, seen };
})();

// The caller's own iterable is walked, and the hit closes it.
globalThis.__sipIterator = (() => {
    const attack = plan(SUPER_ATTACK);
    let opened = 0;
    let closed = 0;
    const once = {
        [Symbol.iterator]() {
            opened += 1;
            let done = false;
            return {
                next: () => (done ? { done: true } : (done = true, { done: false, value: attack })),
                return: () => { closed += 1; return { done: true }; },
            };
        },
    };
    const chosen = potionToSip({ plans: once, held: () => 1, levels: () => ({ base: 70, effective: 70 }) });
    return { sameObject: chosen === attack, opened, closed };
})();

// A hole is a reached plan with no potion, exactly as the frozen loop sees it.
globalThis.__sipHole = (() => {
    const plans = [];
    plans[1] = plan(SUPER_ATTACK);
    const seen = [];
    const message = capture(() => potionToSip({
        plans,
        held: () => { seen.push('held'); return 1; },
        levels: () => { seen.push('levels'); return { base: 70, effective: 70 }; },
    }));
    return { message, seen };
})();

globalThis.__sipMissingList = capture(() => potionToSip({ held: () => 1, levels: () => ({ base: 70, effective: 70 }) }));
globalThis.__sipBadLevels = capture(() => potionToSip({
    plans: [plan(SUPER_ATTACK)],
    held: () => 1,
    levels: () => null,
}));

// A throwing callback propagates and ends the selection.
globalThis.__sipThrows = (() => {
    const log = [];
    const attack = plan(SUPER_ATTACK);
    const strength = plan(SUPER_STRENGTH);
    const levelsBoom = capture(() => potionToSip({
        plans: [attack, strength],
        held: (p) => { log.push('held:' + p.potion.short); return 1; },
        levels: (skill) => { log.push('levels:' + skill); throw new Error('levels boom'); },
    }));
    const heldBoom = capture(() => potionToSip({
        plans: [attack, strength],
        held: (p) => { log.push('held2:' + p.potion.short); throw new Error('held boom'); },
        levels: (skill) => { log.push('levels2:' + skill); return { base: 70, effective: 70 }; },
    }));
    return { levelsBoom, heldBoom, log };
})();

// A callback that runs another selection leaves this one intact.
globalThis.__sipReenter = (() => {
    const log = [];
    const outer = plan(SUPER_ATTACK);
    const inner = plan(SUPER_STRENGTH);
    let innerResult = null;
    const chosen = potionToSip({
        plans: [outer],
        held: () => {
            log.push('outer-held');
            innerResult = potionToSip({
                plans: [inner],
                held: () => { log.push('inner-held'); return 1; },
                levels: (skill) => { log.push('inner-levels:' + skill); return { base: 70, effective: 70 }; },
            });
            return 1;
        },
        levels: (skill) => { log.push('outer-levels:' + skill); return { base: 70, effective: 70 }; },
    });
    return { outerIsOuter: chosen === outer, innerIsInner: innerResult === inner, log };
})();

// --- the native step on its own ----------------------------------------------
globalThis.__wire = (() => {
    const step = (payload) => globalThis.rustyscript.functions.__rs2b0t_boost_potions_step(payload);
    const table = step({ op: 'table' });
    return {
        tableKind: table.kind,
        tableFloor: table.floor,
        tableVial: table.empty_vial,
        rowCount: table.potions.length,
        firstFlask: table.potions[0].flask,
        round: step({ op: 'plan', what: 'round' }),
        roundAfter: step({ op: 'plan', what: 'round', done_potion: 0 }),
        roundEnd: step({ op: 'plan', what: 'round', done_potion: 1 }),
        compare: step({ op: 'plan', what: 'entry', potion: 0 }),
        compared: step({ op: 'plan', what: 'entry', potion: 1, dose: 0, item: 'super strength(3)' }),
        accepted: step({ op: 'plan', what: 'entry', potion: 1, dose: 3, item: 'super strength(1)' }),
        skipped: step({ op: 'plan', what: 'entry', potion: 0, dose: 3, item: 'lobster' }),
        fallback: step({ op: 'plan', what: 'exhausted', potion: 0 }),
        sipStart: step({ op: 'sip', reached: true }),
        sipLevels: step({ op: 'sip', reached: true, base: 70, effective: 77 }),
        sipHit: step({ op: 'sip', reached: true, base: 70, effective: 77, held_ok: true }),
        sipNone: step({ op: 'sip', reached: false }),
        tagged: step({ op: 'faded', base: 70, effective: 'Infinity' }),
        unknown: step({ op: 'nope' }),
    };
})();

// Every decision goes through the binding: no helper answers from JS.
// `rustyscript` is frozen and its `functions` proxy mints a fresh closure per
// read, so the spy swaps the global for an equivalent object.
globalThis.__bridge = (() => {
    const original = globalThis.rustyscript;
    let installed = false;
    let calls = 0;
    const spy = (payload) => {
        calls += 1;
        return original.functions.__rs2b0t_boost_potions_step(payload);
    };
    try {
        globalThis.rustyscript = {
            ...original,
            functions: new Proxy({}, {
                get(_target, name) {
                    return name === '__rs2b0t_boost_potions_step' ? spy : original.functions[name];
                },
            }),
        };
        installed = globalThis.rustyscript !== original;
    } catch (e) {
        installed = false;
    }
    const plans = plannedPotions([]);
    const chosen = potionToSip({ plans, held: () => 1, levels: () => ({ base: 70, effective: 70 }) });
    const faded = boostFaded(70, 70);
    if (installed) globalThis.rustyscript = original;
    return {
        installed,
        calls,
        flasks: plans.map(p => p.flask),
        choseTheCallersPlan: chosen === plans[0],
        faded,
    };
})();

export default class T extends LoopingBot {
    loop() {}
}
"#;

fn spawn() -> LoadIsolate {
    LoadIsolate::spawn(SRC.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

#[test]
fn exports_are_the_frozen_descriptor_rows() {
    let iso = spawn();
    assert_eq!(
        iso.probe("__exports").unwrap(),
        serde_json::json!({
            "floor": 0.1,
            "vial": "Vial",
            "order": ["Att", "Str"],
            "flasks": ["Super attack(3)", "Super strength(3)"],
            "skills": ["attack", "strength"],
            "attackIdentity": true,
            "strengthIdentity": true,
            "attackDoses": [
                "Super attack(4)",
                "Super attack(3)",
                "Super attack(2)",
                "Super attack(1)"
            ],
            "strengthDoses": [
                "Super strength(4)",
                "Super strength(3)",
                "Super strength(2)",
                "Super strength(1)"
            ],
        })
    );
    iso.join();
}

#[test]
fn boost_faded_keeps_the_threshold_drained_and_default_floor_semantics() {
    let iso = spawn();
    assert_eq!(
        iso.probe("__faded").unwrap(),
        serde_json::json!({
            "unboosted": true,
            "onThreshold": true,
            "aboveThreshold": false,
            "fresh": false,
            "levelOne": [true, false],
            "unread": false,
            "drained": false,
            "defaultFloor": true,
            "explicitFloor": [true, false, false],
            "strings": true,
            "nonFinite": [false, false, false],
        }),
        "the arithmetic is the native one; only the Number() conversion stays in JS"
    );
    iso.join();
}

#[test]
fn planned_potions_take_the_first_matching_dose_and_default_the_rest() {
    let iso = spawn();
    assert_eq!(
        iso.probe("__plans").unwrap(),
        serde_json::json!({
            "defaults": [
                {"flask": "Super attack(3)", "want": 1, "short": "Att"},
                {"flask": "Super strength(3)", "want": 1, "short": "Str"},
            ],
            "named": [
                {"flask": "Super attack(4)", "want": 3, "short": "Att"},
                {"flask": "Super strength(3)", "want": 1, "short": "Str"},
            ],
            "trimmedCase": [
                {"flask": "Super attack(2)", "want": 5, "short": "Att"},
                {"flask": "Super strength(3)", "want": 1, "short": "Str"},
            ],
            "unrelated": [
                {"flask": "Super attack(3)", "want": 1, "short": "Att"},
                {"flask": "Super strength(3)", "want": 1, "short": "Str"},
            ],
            "firstMatch": [
                {"flask": "Super attack(1)", "want": 7, "short": "Att"},
                {"flask": "Super strength(3)", "want": 1, "short": "Str"},
            ],
            "bothNamed": [
                {"flask": "Super attack(4)", "want": 2, "short": "Att"},
                {"flask": "Super strength(2)", "want": 4, "short": "Str"},
            ],
            "order": ["Super attack(3)", "Super strength(4)"],
            "countPassthrough": "lots",
        }),
        "attack is planned before strength, and the caller's count is passed through"
    );
    assert_eq!(
        iso.probe("__planIdentity").unwrap(),
        serde_json::json!({
            "attack": true,
            "strength": true,
            "namedIsTheDescriptor": true,
            "dosesAreTheExportedOnes": true,
        })
    );
    assert_eq!(
        iso.probe("__planEarlyExit").unwrap(),
        serde_json::json!([
            {"flask": "Super attack(4)", "want": 1, "short": "Att"},
            {"flask": "Super strength(2)", "want": 3, "short": "Str"},
        ]),
        "each potion's scan stops at its own first match: the third entry is never read"
    );
    assert_eq!(
        iso.probe("__planIterator").unwrap(),
        "entry two",
        "an unmatched scan keeps reading the caller's own entries"
    );
    assert_eq!(
        iso.probe("__planReads").unwrap(),
        7,
        "three comparisons find the attack dose and four miss for strength"
    );
    let errors = iso.probe("__planErrors").unwrap();
    let message = |key: &str| {
        errors
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    assert!(message("notIterable").contains("not iterable"), "{errors}");
    assert!(message("nullEntry").contains("null"), "{errors}");
    assert!(message("nonStringItem").contains("trim"), "{errors}");
    iso.join();
}

#[test]
fn potion_to_sip_asks_levels_then_held_and_returns_the_callers_plan() {
    let iso = spawn();
    assert_eq!(
        iso.probe("__sip").unwrap(),
        serde_json::json!({
            "chosenIsStrength": true,
            "log": ["levels:attack", "held:Att", "levels:strength", "held:Str"],
        }),
        "a held potion is skipped and the later plan is probed in the same order"
    );
    assert_eq!(
        iso.probe("__sipOrder").unwrap(),
        serde_json::json!({
            "chosenIsAttack": true,
            "log": ["levels:attack", "held:Att"],
        }),
        "attack wins a tick both could use, and the hit ends the walk"
    );
    assert_eq!(
        iso.probe("__sipHeld").unwrap(),
        serde_json::json!({
            "chosenIsStrength": true,
            "log": ["levels:attack", "held:Att", "levels:strength", "held:Str"],
        }),
        "an empty pack still asks levels before held"
    );
    assert_eq!(
        iso.probe("__sipNone").unwrap(),
        serde_json::json!({
            "heldBoost": null,
            "emptyPack": null,
            "drained": null,
            "emptyPlans": null,
            "levels": ["levels", "levels", "levels"],
        }),
        "a held boost, an empty pack and a drained skill all sip nothing"
    );
    assert_eq!(
        iso.probe("__sipCaller").unwrap(),
        serde_json::json!({
            "sameObject": true,
            "skillReads": 1,
            "seen": ["levels:ranged", "held"],
        }),
        "the caller's own plan decides the skill and is returned unchanged"
    );
    assert_eq!(
        iso.probe("__sipIterator").unwrap(),
        serde_json::json!({"sameObject": true, "opened": 1, "closed": 1}),
        "the caller's iterator is opened once and closed by the hit"
    );
    let hole = iso.probe("__sipHole").unwrap();
    assert!(
        hole["message"]
            .as_str()
            .unwrap_or("")
            .contains("reading 'potion'"),
        "a hole reaches the body as undefined: {hole}"
    );
    assert_eq!(hole["seen"], serde_json::json!([]));
    assert!(
        iso.probe("__sipMissingList")
            .unwrap()
            .as_str()
            .unwrap_or("")
            .contains("not iterable"),
        "a missing plan list is not iterable"
    );
    assert!(
        iso.probe("__sipBadLevels")
            .unwrap()
            .as_str()
            .unwrap_or("")
            .contains("null"),
        "the frozen destructuring of a null levels answer still throws"
    );
    assert_eq!(
        iso.probe("__sipThrows").unwrap(),
        serde_json::json!({
            "levelsBoom": "levels boom",
            "heldBoom": "held boom",
            "log": ["levels:attack", "levels2:attack", "held2:Att"],
        }),
        "a throwing callback propagates and no later plan is probed"
    );
    assert_eq!(
        iso.probe("__sipReenter").unwrap(),
        serde_json::json!({
            "outerIsOuter": true,
            "innerIsInner": true,
            "log": ["outer-levels:attack", "outer-held", "inner-levels:strength", "inner-held"],
        }),
        "a reentrant selection keeps both results and both orders"
    );
    iso.join();
}

#[test]
fn native_step_wire_contract_is_round_accept_and_sip() {
    let iso = spawn();
    assert_eq!(
        iso.probe("__wire").unwrap(),
        serde_json::json!({
            "tableKind": "table",
            "tableFloor": 0.1,
            "tableVial": "Vial",
            "rowCount": 2,
            "firstFlask": "Super attack(3)",
            "round": {"kind": "scan", "potion": 0},
            "roundAfter": {"kind": "scan", "potion": 1},
            "roundEnd": {"kind": "done"},
            "compare": {"kind": "compare", "dose": 0},
            "compared": {"kind": "compare", "dose": 1},
            "accepted": {"kind": "accept", "flask": "Super strength(1)"},
            "skipped": {"kind": "next"},
            "fallback": {"kind": "fallback", "flask": "Super attack(3)", "want": 1},
            "sipStart": {"kind": "levels"},
            "sipLevels": {"kind": "held"},
            "sipHit": {"kind": "hit"},
            "sipNone": {"kind": "none"},
            "tagged": {"kind": "value", "value": false},
            "unknown": {"kind": "error", "feature": "boostPotions"},
        })
    );
    iso.join();
}

#[test]
fn the_helpers_answer_through_the_native_binding() {
    let iso = spawn();
    let bridge = iso.probe("__bridge").unwrap();
    assert_eq!(
        bridge["flasks"],
        serde_json::json!(["Super attack(3)", "Super strength(3)"])
    );
    assert_eq!(bridge["choseTheCallersPlan"], serde_json::json!(true));
    assert_eq!(bridge["faded"], serde_json::json!(true));
    assert_eq!(
        bridge["installed"],
        serde_json::json!(true),
        "the isolate exposes the step binding on `rustyscript.functions`"
    );
    assert_eq!(
        bridge["calls"],
        serde_json::json!(9),
        "two default plans, one sip walk and one faded comparison: {bridge}"
    );
    iso.join();
}
