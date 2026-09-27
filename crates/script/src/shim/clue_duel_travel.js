import { runMachine } from '../../../shim/_kernel.js';

const call = (payload) => globalThis.rustyscript.functions.__rs2b0t_duel(payload);
const tables = call({ op: 'tables' });

export const DUEL_CLUE_ID = tables.clue.id;
export const DUEL_CLUE_TILE = Object.freeze(tables.clue.tile);

export function crossesClueDuel(dest) {
    return call({ op: 'crosses', tile: dest }) === true;
}

export async function walkAcrossClueDuel(dest, radius, log) {
    const out = await runMachine('clue_duel_travel', {
        x: Number(dest?.x),
        z: Number(dest?.z),
        level: Number(dest?.level),
        radius: Number(radius),
    }, { log });
    return out.kind === 'done' && out.value === true;
}
