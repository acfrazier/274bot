import { Input } from '../../input/Input.js';
import { notImplValue, proxy, queue, runMachine } from '../../shim/_kernel.js';

const config = () => (globalThis.__rs2b0t_host || {}).content?.duel || {};
const selected = (name) => Number.isInteger(config()[name])
    ? config()[name]
    : notImplValue('Duel.' + name);
const call = (payload) => globalThis.rustyscript.functions.__rs2b0t_duel(payload);
const facts = () => call({ op: 'facts' }) || {};
const close = async (kind) => {
    const out = await runMachine('duel_close', { kind });
    return out.kind === 'done' && out.value === true;
};

export const DUEL_SELECT_MODAL = selected('select_modal');
export const DUEL_CONFIRM_MODAL = selected('confirm_modal');
export const DUEL_WIN_MODAL = selected('win_modal');
export const DUEL_FIGHT_ARENAS = Object.freeze(
    call({ op: 'tables' }).pens.map((pen) => Object.freeze(pen)),
);

export function parseDuelPartnerHeader(header) {
    return call({ op: 'header', text: typeof header === 'string' ? header : null });
}

// Rust names the pen; the answer is that frozen DUEL_FIGHT_ARENAS entry,
// so a pen compares equal to itself across calls, as the trainer expects.
export function fightArenaAt(tile) {
    const index = call({ op: 'pen', tile });
    return index === null ? null : DUEL_FIGHT_ARENAS[index];
}

export const Duel = proxy('Duel', {
    offerOpen() { return facts().offer === true; },
    confirmOpen() { return facts().confirm === true; },
    winOpen() { return facts().win === true; },
    active() {
        const value = facts();
        return value.offer === true || value.confirm === true;
    },
    partner() { return facts().partner ?? null; },
    waitingForOther() { return facts().waiting === true; },
    challenge(player) { return Input.interactPlayer(player?.index, 1); },
    fight(player) { return Input.interactPlayer(player?.index, 2); },
    accept() {
        const op = call({ op: 'accept' });
        if (!op) return false;
        queue(op);
        return true;
    },
    cancel() { return close('cancel'); },
    closeWin() { return close('closeWin'); },
});
