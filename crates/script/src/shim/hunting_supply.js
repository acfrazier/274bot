// Hunting supply: a name map over the Rust hunt families (`crate::hunt`)
// and the hunting helpers (`crate::hunt_catalog`). Rust owns the loops,
// walks, ops, waits and the bank-trip policy; the flask plans are declared
// data.
import { runMachine } from '../../../shim/_kernel.js';
import { Sustain } from '../../sustain/Sustain.js';
import { EnterLair, bankRoutine as huntBank, hooksOf, leaveLair, siteArgs } from './combat.js';

const logic = (name, ...args) => globalThis.__rs2b0t_hunt_logic(name, ...args);

export const SHIELD = 'Dragonfire shield';
export const POISONED = /you have been poisoned/i;
export const COINS = 'Coins';
export const ANTIPOISON_LABEL = 'Superantipoison';
export const ANTIPOISON_DOSES = [4, 3, 2, 1].map((d) => `${ANTIPOISON_LABEL}(${d})`);
export const PRAYER_LABEL = 'Prayer potion';
export const PRAYER_DOSES = [4, 3, 2, 1].map((d) => `${PRAYER_LABEL}(${d})`);
export const ANTIFIRE_LABEL = 'Antifire potion';
export const ANTIFIRE_DOSES = [4, 3, 2, 1].map((d) => `${ANTIFIRE_LABEL}(${d})`);

export const antipoisonPlan = (want) => ({ flask: ANTIPOISON_DOSES[0], doses: ANTIPOISON_DOSES, want });
export const prayerPlan = (want) => ({ flask: PRAYER_DOSES[0], doses: PRAYER_DOSES, want });
export const antifirePlan = (want) => ({ flask: ANTIFIRE_DOSES[0], doses: ANTIFIRE_DOSES, want });

export function doseToDrink(count, doses = ANTIPOISON_DOSES) {
    return globalThis.__rs2b0t_hunt_dose(count, [...doses]);
}

export const escapeRunesFor = (teleportId) => logic('escapeRunesFor', teleportId);
export const inCell = () => logic('inCell');

// One EnterLair session per site key.
const enters = new Map();
function enterFor(h, site) {
    let enter = enters.get(site.key);
    if (!enter) {
        enter = new EnterLair(h, site);
        enters.set(site.key, enter);
    }
    return enter;
}

export async function enterLair(h, site) {
    return enterFor(h, site).execute();
}

// Frozen module-level `feePaidFor`, kept by Rust across ResetSession.
export function feePrepaid(site) {
    return globalThis.__rs2b0t_hunt('feePrepaid', 'hunt-enter', String(site?.key ?? '')) === true;
}

export { leaveLair };

export async function acquireKey(h, site) {
    const out = await runMachine('hunt-acquire', { site: siteArgs(site) }, hooksOf(h, site));
    return out.kind === 'done' ? out.value : logic('keyState', site.keyItem?.id ?? null);
}

// Frozen `leaveCell` only opens the cell door from inside.
export async function leaveCell(h) {
    const out = await runMachine('hunt-cell', { site: siteArgs({ key: 'leave-cell' }, { leaveOnly: true }) }, hooksOf(h, null));
    return out.kind === 'done' && out.value === true;
}

export async function teleportOut(h, site) {
    const out = await runMachine('hunt-teleport-out', { id: String(site.escapeTeleportId ?? '') }, hooksOf(h, site));
    return out.kind === 'done' ? out.value : 'the cast never landed';
}

// Frozen `waitFed`: `cond` each tick, pumping Sustain between polls.
export async function waitFed(cond, ms) {
    const out = await runMachine('hunt-wait-fed', { ms: Number(ms) }, { cond, sustain: () => Sustain.run() });
    return out.kind === 'done' && out.value === true;
}

// Frozen `BankOpts` cross as they are; Rust owns the defaults and the math.
export async function bankRoutine(h, site, opts) {
    await huntBank(h, site, opts);
}

export function walkApproach() {
    throw new Error('not impl: walkApproach');
}

export function withdrawTo() {
    throw new Error('not impl: withdrawTo');
}
