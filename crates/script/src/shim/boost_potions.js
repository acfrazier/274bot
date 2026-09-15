// Boost-potion descriptors, planning and sip selection.
//
// The descriptor rows, the table order, the planning defaults, the boost floor
// and every selection decision live in `script::boost_potions`. This shim only
// marshals arguments, runs the one caller iteration / caller callback the native
// step asks for, and returns the original objects: the exported descriptors, the
// caller's own plan objects and the caller's carry counts.
import { notImpl } from '../../shim/_kernel.js';

function call(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_boost_potions_step(payload);
}

function fail(step) {
    throw notImpl(step && step.feature ? step.feature : 'boostPotions');
}

/** A JS number as the native step reads it: a non-finite value keeps its `String(value)` tag. */
function jsNumber(value) {
    const n = Number(value);
    return Number.isFinite(n) ? n : String(n);
}

const TABLE = call({ op: 'table' });
if (!TABLE || TABLE.kind !== 'table') fail(TABLE);

export const BOOST_POTIONS = TABLE.potions.map(potion => ({
    skill: potion.skill,
    short: potion.short,
    flask: potion.flask,
    doses: potion.doses,
}));

export const SUPER_ATTACK = BOOST_POTIONS[0];
export const SUPER_STRENGTH = BOOST_POTIONS[1];
export const EMPTY_VIAL = TABLE.empty_vial;
export const BOOST_FLOOR = TABLE.floor;

/** Whether the boost has decayed back into the floor band. */
export function boostFaded(base, effective, floor) {
    const payload = { op: 'faded', base: jsNumber(base), effective: jsNumber(effective) };
    // An omitted third argument is the frozen default parameter, not a caller number.
    if (floor !== undefined) payload.floor = jsNumber(floor);
    const step = call(payload);
    if (step.kind !== 'value') fail(step);
    return step.value;
}

/** The potions to carry, taking the dose form and count from the loadout and falling back to one 3-dose flask of each. */
export function plannedPotions(carry) {
    const plans = [];
    let round = call({ op: 'plan', what: 'round' });
    while (round.kind === 'scan') {
        const potion = round.potion;
        let planned = false;
        for (const entry of carry) {
            // The frozen `find` re-reads the item once per dose comparison.
            let step = call({ op: 'plan', what: 'entry', potion });
            while (step.kind === 'compare') {
                step = call({ op: 'plan', what: 'entry', potion, dose: step.dose, item: entry.item.trim().toLowerCase() });
            }
            if (step.kind === 'accept') {
                plans.push({ potion: BOOST_POTIONS[potion], flask: step.flask, want: entry.qty });
                planned = true;
                break; // the frozen `return` closes the caller's iterator the same way
            }
            if (step.kind !== 'next') fail(step);
        }
        if (!planned) {
            const step = call({ op: 'plan', what: 'exhausted', potion });
            if (step.kind !== 'fallback') fail(step);
            plans.push({ potion: BOOST_POTIONS[potion], flask: step.flask, want: step.want });
        }
        round = call({ op: 'plan', what: 'round', done_potion: potion });
    }
    if (round.kind !== 'done') fail(round);
    return plans;
}

/** The one potion to drink this tick, or null. */
export function potionToSip(s) {
    for (const plan of s.plans) {
        const step = call({ op: 'sip', reached: true });
        if (step.kind !== 'levels') fail(step);
        const { base, effective } = s.levels(plan.potion.skill);
        const levels = { op: 'sip', reached: true, base: jsNumber(base), effective: jsNumber(effective) };
        const held = call(levels);
        if (held.kind !== 'held') fail(held);
        const decided = call({ ...levels, held_ok: s.held(plan) > 0 });
        if (decided.kind === 'hit') return plan;
        if (decided.kind !== 'next') fail(decided);
    }
    const step = call({ op: 'sip', reached: false });
    if (step.kind !== 'none') fail(step);
    return null;
}
