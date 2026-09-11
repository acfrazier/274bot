import { SPELL_DB } from '../../data/spelldb.js';

export const ATTACKSTYLE_MAGIC_VARP = 108;
export const AUTOCAST_ARMED = 3;

function hostFn(name) {
    return globalThis.rustyscript.functions[name];
}

function remainingCosts(spellName, wielded) {
    return hostFn('__rs2b0t_runes_per_cast')(
        String(spellName || ''),
        Array.isArray(wielded) ? wielded.map((item) => String(item)) : [],
    );
}

export function runesPerCast(spellName, wielded) {
    return remainingCosts(spellName, wielded);
}

export function spellButtonCom(spellName) {
    return hostFn('__rs2b0t_spell_button_com')(String(spellName || ''));
}

export function castsAvailable(spellName, wielded, held) {
    const costs = remainingCosts(spellName, wielded);
    if (!costs) {
        return 0;
    }
    if (costs.length === 0) {
        return Number.POSITIVE_INFINITY;
    }
    return Math.min(...costs.map((cost) => Math.floor(Number(held(cost.rune)) / cost.count)));
}

export function runeWithdrawList(spellName, wielded, casts) {
    const costs = remainingCosts(spellName, wielded);
    if (!costs) {
        return [];
    }
    const n = Number(casts);
    return costs.map((cost) => ({ rune: cost.rune, count: cost.count * n }));
}

export { SPELL_DB };
