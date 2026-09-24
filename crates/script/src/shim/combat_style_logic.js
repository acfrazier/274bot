import { SPELL_DB } from '../../data/spelldb.js';

export const ATTACKSTYLE_MAGIC_VARP = 108;
export const AUTOCAST_ARMED = 3;

function hostFn(name) {
    return globalThis.rustyscript.functions[name];
}

function marshalled(wielded) {
    return Array.isArray(wielded) ? wielded.map((item) => String(item)) : [];
}

export function runesPerCast(spellName, wielded) {
    return hostFn('__rs2b0t_runes_per_cast')(String(spellName || ''), marshalled(wielded));
}

export function spellButtonCom(spellName) {
    return hostFn('__rs2b0t_spell_button_com')(String(spellName || ''));
}

export function castsAvailable(spellName, wielded, held) {
    return globalThis.__rs2b0t_combat_style(
        'castsAvailable', String(spellName || ''), marshalled(wielded), held,
    );
}

export function runeWithdrawList(spellName, wielded, casts) {
    return globalThis.__rs2b0t_combat_style(
        'runeWithdrawList', String(spellName || ''), marshalled(wielded), casts,
    );
}

export { SPELL_DB };
