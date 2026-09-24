// Hunt Tasks and runs: a name map over the Rust hunt families
// (`crate::hunt`). Rust reads the scene, owns the policy, the walks, the
// game ops, the waits and the retries; the site crosses once; the
// CombatHost methods are handed over as hooks Rust calls. A Task class
// keeps one token: `validate` is one synchronous call, `execute` awaits
// one machine run.
import { notImpl, runMachine } from '../../../shim/_kernel.js';
import { Sustain } from '../../sustain/Sustain.js';

function hunt(op, family, a, b) {
    const fn = globalThis.__rs2b0t_hunt;
    if (typeof fn !== 'function') throw notImpl('hunt');
    return fn(op, family, a, b);
}

const tile = (t) => (t ? { x: t.x, z: t.z, level: t.level ?? 0 } : null);
const tiles = (rows) => (rows || []).map(tile);
const phrase = (v) => (!v ? '' : typeof v === 'string' ? v : typeof v.source === 'string' ? v.source : String(v));

// The site's data, once; its `inArea` predicate stays the script's own hook.
export function siteArgs(site, extra) {
    const s = site || {};
    const talk = s.talkGate || null;
    const fee = s.feeGate || null;
    const gate = s.gate || null;
    const exit = s.exit || null;
    return {
        key: s.key,
        target: s.target,
        alsoHunt: s.alsoHunt || [],
        safespots: tiles(s.safespots),
        meleeAnchor: tile(s.meleeAnchor),
        approach: tiles(s.approach),
        boxes: s.boxes || [],
        fireAtRange: s.fireAtRange === true,
        rangedThreat: s.rangedThreat === true,
        bank: tile(s.bank),
        keyItem: s.keyItem ?? null,
        coins: s.coins ?? null,
        escapeTeleportId: s.escapeTeleportId ?? null,
        walkOut: tile(s.walkOut),
        talkGate: talk && { npc: talk.npc, op: talk.op, choose: phrase(talk.choose), stand: tile(talk.stand) },
        feeGate: fee && {
            npc: fee.npc,
            op: fee.op,
            coins: fee.coins,
            stand: tile(fee.stand),
            entrance: fee.entrance || null,
            paidLine: phrase(fee.paidLine),
            prepaidLine: phrase(fee.prepaidLine),
        },
        gate: gate && { locId: gate.locId, op: gate.op, outside: tile(gate.outside), inside: tile(gate.inside) },
        exit: exit && { locId: exit.locId, op: exit.op, stand: tile(exit.stand) },
        route: s.route || null,
        outLever: s.outLever || null,
        upLadder: s.upLadder || null,
        ...extra,
    };
}

const method = (host, name) => (typeof host?.[name] === 'function' ? host[name].bind(host) : undefined);

// The CombatHost / JiveHost surface as the hooks the families call.
export function hooksOf(host, site, leave) {
    const h = host || {};
    const hooks = { leave, sustain: () => Sustain.run() };
    for (const name of [
        'log', 'vlog', 'setStatus', 'eatOnce', 'armSpecial', 'countBurial', 'setSafespotIndex',
        'pickWeapon', 'parkFor', 'countBankTrip', 'hpFraction', 'panicHp', 'retreatHp', 'hasFood',
        'needEat', 'style', 'safespotIndex', 'buryBones', 'boneName', 'shieldReady', 'leaveByWalk',
        'foodName', 'foodWithdraw', 'weaponName', 'ammoName', 'spellName', 'keepExtra',
    ]) {
        hooks[name] = method(h, name);
    }
    hooks.died = () => h.died === true;
    hooks.parked = () => h.parked === true;
    hooks.targetIdx = () => h.targetIdx ?? null;
    hooks.setTarget = (index) => {
        h.targetIdx = index ?? null;
    };
    if (typeof site?.inArea === 'function') hooks.inArea = (t) => site.inArea(t);
    return hooks;
}

// Frozen `anchorFor`: the melee anchor, or the clamped safespot.
export function anchorFor(site, style, index) {
    if (style === 'melee') return site.meleeAnchor;
    return site.safespots[Math.min(index, site.safespots.length - 1)] ?? site.meleeAnchor;
}

class HuntTask {
    constructor(family, host, site) {
        this.family = family;
        this.host = host;
        this.site = site;
        this.hooks = hooksOf(host, site);
        this.token = hunt('begin', family, siteArgs(site));
    }

    validate() {
        return hunt('validate', this.family, this.token, this.hooks) === true;
    }

    async run() {
        const out = await runMachine(this.family, { token: this.token }, this.hooks);
        return out.kind === 'done' ? out.value : undefined;
    }
}

export class Fight extends HuntTask {
    constructor(host, site) {
        super('hunt-fight', host, site);
        if (host) host.fight = this;
    }

    async execute() {
        await this.run();
    }

    reset() {
        hunt('reset', this.family, this.token);
    }

    interruptWatch() {
        hunt('interruptWatch', this.family, this.token);
    }

    blocksLoot() {
        return hunt('blocksLoot', this.family, this.token, this.hooks) === true;
    }
}

export class Retreat extends HuntTask {
    constructor(host, site) {
        super('hunt-retreat', host, site);
    }

    async execute() {
        this.host.fight?.interruptWatch();
        await this.run();
    }
}

export class HoldSafespot extends HuntTask {
    constructor(host, site) {
        super('hunt-hold', host, site);
    }

    async execute() {
        this.host.fight?.interruptWatch();
        await this.run();
    }
}

export class WalkToSpot extends HuntTask {
    constructor(host, site) {
        super('hunt-walkspot', host, site);
    }

    async execute() {
        this.host.fight?.interruptWatch();
        await this.run();
    }
}

export class EnterLair extends HuntTask {
    constructor(host, site) {
        super('hunt-enter', host, site);
    }

    async execute() {
        return (await this.run()) === true;
    }
}

async function run(family, host, site, extra, leave) {
    const out = await runMachine(family, { site: siteArgs(site, extra) }, hooksOf(host, site, leave));
    return out.kind === 'done' && out.value === true;
}

export function cell(host, site) {
    return run('hunt-cell', host, site);
}

export function leaveLair(host, site) {
    return run('hunt-leave', host, site);
}

export async function acquireKey(host, site) {
    const out = await runMachine('hunt-acquire', { site: siteArgs(site) }, hooksOf(host, site));
    return out.kind === 'done' ? out.value : undefined;
}

export function bankRoutine(host, site, opts) {
    const o = opts || {};
    const extra = {
        withdrawFood: o.withdrawFood === true,
        wear: o.wear || [],
        carry: o.carry || [],
        runes: o.runes || [],
        escapeRunes: o.escapeRunes || [],
        flasks: o.flasks || [],
        healTo: o.healTo ?? null,
        ammoWant: o.ammo ?? null,
    };
    const leave = typeof o.leave === 'function' ? () => o.leave(host, site) : undefined;
    return run('hunt-bank', host, site, extra, leave);
}
