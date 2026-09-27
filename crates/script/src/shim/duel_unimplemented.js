import { actions } from '../../adapter/ClientAdapter.js';
import { Input } from '../../input/Input.js';
import { notImplValue, proxy } from '../../shim/_kernel.js';

const config = () => (globalThis.__rs2b0t_host || {}).content?.duel || {};
const selected = (name) => Number.isInteger(config()[name])
    ? config()[name]
    : notImplValue('Duel.' + name);
const call = (payload) => globalThis.rustyscript.functions.__rs2b0t_duel(payload);
const facts = () => call({ op: 'facts' }) || {};

export const DUEL_SELECT_MODAL = selected('select_modal');
export const DUEL_CONFIRM_MODAL = selected('confirm_modal');
export const DUEL_WIN_MODAL = selected('win_modal');
export const DUEL_FIGHT_ARENAS = Object.freeze([
    Object.freeze({ minX: 3333, maxX: 3357, minZ: 3244, maxZ: 3258 }),
    Object.freeze({ minX: 3364, maxX: 3388, minZ: 3225, maxZ: 3239 }),
    Object.freeze({ minX: 3333, maxX: 3357, minZ: 3206, maxZ: 3220 }),
    Object.freeze({ minX: 3364, maxX: 3388, minZ: 3244, maxZ: 3258 }),
    Object.freeze({ minX: 3333, maxX: 3357, minZ: 3225, maxZ: 3239 }),
    Object.freeze({ minX: 3364, maxX: 3388, minZ: 3206, maxZ: 3220 }),
]);

export function parseDuelPartnerHeader(header) {
    return call({ op: 'header', text: typeof header === 'string' ? header : null });
}

export function fightArenaAt(tile) {
    return call({ op: 'pen', tile });
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
        const value = facts();
        if (value.offer === true) return actions.ifButton(selected('select_accept'));
        if (value.confirm === true) return actions.ifButton(selected('confirm_accept'));
        return false;
    },
    async cancel() {
        if (!this.active()) return false;
        return actions.closeModal();
    },
    async closeWin() {
        if (!this.winOpen()) return false;
        return actions.closeModal();
    },
});
