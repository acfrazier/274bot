import { runMachine } from '../../../shim/_kernel.js';
import { SettingsStore } from '../../../runtime/Settings.js';
import { reader } from '../../../adapter/ClientAdapter.js';

export const DUEL_CLUE_ID = 3554;
export const DUEL_CLUE_TILE = Object.freeze({ x: 3374, z: 3250, level: 0 });

const call = (payload) => globalThis.rustyscript.functions.__rs2b0t_duel(payload);

export function crossesClueDuel(dest) {
    return call({ op: 'crosses', tile: dest }) === true;
}

export async function walkAcrossClueDuel(dest, radius, log) {
    const partner = SettingsStore.globalBag().str('clueDuelPartner', '').trim();
    const selfName = reader.localPlayerName();
    if (call({ op: 'valid-partner', partner, self_name: selfName }) !== true) {
        if (typeof log === 'function') {
            log('set Global clue duel partner and run that account in Duel Arena Clue helper mode');
        }
        return false;
    }
    const out = await runMachine('clue_duel_travel', {
        partner,
        self_name: selfName,
        x: Number(dest?.x),
        z: Number(dest?.z),
        level: Number(dest?.level),
        radius: Number(radius),
        leave_only: false,
        allow_teleports: true,
        allow_wilderness: true,
        allow_bank_fetch: true,
    }, {});
    return out.kind === 'done' && out.value === true;
}
