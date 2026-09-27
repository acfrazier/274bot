import { runMachine } from '../../shim/_kernel.js';

const call = (payload) => globalThis.rustyscript.functions.__rs2b0t_duel(payload);
const tables = call({ op: 'tables' });

export const CLUE_DUEL_LOBBY = Object.freeze(tables.lobby);
export const CLUE_DUEL_OPTIONS = tables.rules;

export function clueDuelName(name) {
    return call({ op: 'name', name: typeof name === 'string' ? name : '' });
}

export async function leaveClueDuel(log) {
    const out = await runMachine('clue_duel_travel', { leave_only: true }, { log });
    return out.kind === 'done' && out.value === true;
}

export class ClueDuelHandshake {
    constructor(partner, initiator, log) {
        this.partner = String(partner ?? '');
        this.initiator = initiator === true;
        this.log = log;
        call({ op: 'handshake-new', partner: this.partner });
    }

    async tick() {
        const out = await runMachine('clue_duel_handshake', {
            partner: this.partner,
            initiator: this.initiator,
        }, { log: this.log });
        return out.kind === 'done' && out.value === true;
    }
}

export class ClueDuelHelper {
    constructor(partner, log) {
        this.partner = String(partner ?? '');
        this.log = log;
    }

    validate() { return true; }

    async execute() {
        await runMachine('clue_duel_helper', { partner: this.partner }, { log: this.log });
    }
}
