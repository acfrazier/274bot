// Hunting supply: a name map over the Rust hunt families (`crate::hunt`)
// and the hunting helpers (`crate::hunt_catalog`). Rust owns the loops,
// walks, ops and waits; the flask plans are declared data.
import { runMachine } from '../../../shim/_kernel.js';
import { Equipment } from '../../equipment/Equipment.js';
import { runeWithdrawList } from '../CombatStyleLogic.js';
import { EnterLair, bankRoutine as huntBank, cell, hooksOf, leaveLair, siteArgs } from './combat.js';

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

// One EnterLair session per site key: the frozen module-level fee proof
// lives on its token.
const enters = new Map();
function enterFor(h, site) {
    let enter = enters.get(site.key);
    if (!enter) {
        enter = new EnterLair(h, site);
        enters.set(site.key, enter);
    }
    enter.host = h;
    return enter;
}

export async function enterLair(h, site) {
    return enterFor(h, site).execute();
}

export function feePrepaid(site) {
    const enter = enters.get(site.key);
    return !!enter && globalThis.__rs2b0t_hunt('feePrepaid', 'hunt-enter', enter.token, site.key) === true;
}

export { leaveLair };

export async function acquireKey(h, site) {
    const out = await runMachine('hunt-acquire', { site: siteArgs(site) }, hooksOf(h, site));
    return out.kind === 'done' ? out.value : logic('keyState', site.keyItem?.id ?? null);
}

// Frozen leaveCell only opens the door from inside.
export function leaveCell(h) {
    const site = { key: 'taverley-blue', keyItem: { name: 'Dusty key', id: 1590 } };
    return runMachine('hunt-cell', { site: siteArgs(site, { leaveOnly: true }) }, hooksOf(h, site)).then(
        (out) => out.kind === 'done' && out.value === true,
    );
}

export async function teleportOut(h, site) {
    const out = await runMachine('hunt-teleport-out', { id: String(site.escapeTeleportId ?? '') }, hooksOf(h, site));
    return out.kind === 'done' ? out.value : 'the cast never landed';
}

export async function waitFed(cond, ms) {
    const out = await runMachine('hunt-wait-fed', { ms: Number(ms) }, { cond });
    return out.kind === 'done' && out.value === true;
}

const asFlask = (plan) => ({ flask: plan.flask, doses: plan.potion.doses, want: plan.want });

// Frozen BankOpts coerced to the bank family's loadout.
export async function bankRoutine(h, site, opts) {
    const o = opts || {};
    const style = typeof h.style === 'function' ? h.style() : '';
    const wielded = Equipment.items().map((i) => i.name ?? '');
    const runes = style === 'mage'
        ? runeWithdrawList(h.spellName(), wielded, o.runeCasts ?? 150).map((r) => ({ name: r.rune, count: r.count + (o.runeBuffer ?? 300) }))
        : [];
    const escape = escapeRunesFor(site.escapeTeleportId).runes
        .map((r) => ({ name: r.rune, count: r.count * ((o.escapeStock ?? 2) + 1) }));
    await huntBank(h, site, {
        withdrawFood: o.withdrawFood === true,
        wear: o.wear,
        carry: o.carry,
        runes,
        escapeRunes: escape,
        flasks: [...(o.potions ?? []).map(asFlask), ...(o.flasks ?? [])],
        healTo: o.healTo,
        ammo: style === 'range' ? (o.ammo ?? 500) : undefined,
        leave: o.leave,
    });
}

export function walkApproach() {
    throw new Error('not impl: walkApproach');
}

export function withdrawTo() {
    throw new Error('not impl: withdrawTo');
}
