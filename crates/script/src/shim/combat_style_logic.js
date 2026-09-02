import { SPELL_DB } from '../../data/spelldb.js';
import { notV1 } from '../../shim/_kernel.js';

export const ATTACKSTYLE_MAGIC_VARP = 108;
export const AUTOCAST_ARMED = 3;

export function runesPerCast(_spellName, _wielded) {
    throw notV1('runesPerCast');
}

export function spellButtonCom(_spellName) {
    throw notV1('spellButtonCom');
}

export function castsAvailable(_spellName, _wielded, _held) {
    throw notV1('castsAvailable');
}

export function runeWithdrawList(_spellName, _wielded, _casts) {
    throw notV1('runeWithdrawList');
}

export { SPELL_DB };
