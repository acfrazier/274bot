import { runMachine } from '../../shim/_kernel.js';

export const CLUE_DUEL_LOBBY = Object.freeze({ x: 3368, z: 3274, level: 0 });
export const CLUE_DUEL_OPTIONS = 1024;

const call = (payload) => globalThis.rustyscript.functions.__rs2b0t_duel(payload);

export function clueDuelName(name) {
    return call({ op: 'name', name: typeof name === 'string' ? name : '' });
}

export async function leaveClueDuel(_log) {
    const out = await runMachine('clue_duel_travel', {
        partner: '', x: 0, z: 0, level: 0, radius: 0, leave_only: true,
    }, {});
    return out.kind === 'done' && out.value === true;
}

export class ClueDuelHandshake {
    constructor(partner, initiator, _log) {
        this.partner = String(partner ?? '');
        this.initiator = initiator === true;
    }

    async tick() {
        const out = await runMachine('clue_duel_handshake', {
            partner: this.partner,
            initiator: this.initiator,
        }, {});
        return out.kind === 'done' && out.value === true;
    }
}

export class ClueDuelHelper {
    constructor(partner, _log) {
        this.partner = String(partner ?? '');
    }

    validate() { return true; }

    async execute() {
        await runMachine('clue_duel_helper', { partner: this.partner }, {});
    }
}
