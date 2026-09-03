import { SPELL_DB } from '../../data/spelldb.js';
import { notImpl } from '../../shim/_kernel.js';

export const ATTACKSTYLE_MAGIC_VARP = 108;
export const AUTOCAST_ARMED = 3;

export function runesPerCast(_spellName, _wielded) {
    throw notImpl('runesPerCast');
}

export function spellButtonCom(_spellName) {
    throw notImpl('spellButtonCom');
}

export function castsAvailable(_spellName, _wielded, _held) {
    throw notImpl('castsAvailable');
}

export function runeWithdrawList(_spellName, _wielded, _casts) {
    throw notImpl('runeWithdrawList');
}

export { SPELL_DB };
