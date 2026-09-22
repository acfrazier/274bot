// Thin Fight Task. Rust owns field pick, clocks and gates. This file
// marshals CombatHost + site data and dispatches the effect table only.
import { notImpl, queue } from '../../../shim/_kernel.js';
import { Execution } from '../../execution/Execution.js';
import { Sustain } from '../../sustain/Sustain.js';

function call(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_fight
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('Fight');
    }
    return fn(payload);
}

function tile(t) {
    if (!t) return null;
    return { x: t.x, z: t.z, level: t.level ?? 0 };
}

function projection(host, site) {
    return {
        died: host.died === true,
        targetIdx: host.targetIdx ?? null,
        hpFraction: host.hpFraction(),
        panicHp: host.panicHp(),
        retreatHp: host.retreatHp(),
        hasFood: host.hasFood(),
        needEat: host.needEat(),
        style: host.style(),
        safespotIndex: host.safespotIndex(),
        buryBones: host.buryBones(),
        boneName: host.boneName(),
        hasVlog: typeof host.vlog === 'function',
        hasArmSpecial: typeof host.armSpecial === 'function',
        hasShieldReady: typeof host.shieldReady === 'function',
        shieldReady: typeof host.shieldReady === 'function' ? host.shieldReady() === true : false,
        key: site.key,
        target: site.target,
        alsoHunt: site.alsoHunt || [],
        safespots: (site.safespots || []).map(tile),
        meleeAnchor: tile(site.meleeAnchor),
        boxes: site.boxes || [],
        fireAtRange: site.fireAtRange === true,
        rangedThreat: site.rangedThreat === true,
        approach: (site.approach || []).map(tile),
    };
}

export class Fight {
    constructor(host, site) {
        this.host = host;
        this.site = site;
        const started = call({ op: 'begin' });
        this.token = started.token;
        if (host) host.fight = this;
    }

    validate() {
        const out = call({ op: 'validate', token: this.token, ...projection(this.host, this.site) });
        return out === true || out?.value === true;
    }

    async execute() {
        let reply = null;
        for (;;) {
            const step = call({
                op: 'next',
                token: this.token,
                reply,
                ...projection(this.host, this.site),
            });
            reply = null;
            if (!step || step.kind === 'yield' || step.kind === 'aborted') {
                return;
            }
            switch (step.kind) {
                case 'log':
                    this.host.log?.(step.message);
                    break;
                case 'vlog':
                    this.host.vlog?.(step.message);
                    break;
                case 'status':
                    this.host.setStatus?.(step.message);
                    break;
                case 'eat':
                    reply = { eatOk: !!(await this.host.eatOnce()) };
                    break;
                case 'arm-special':
                    await this.host.armSpecial?.();
                    break;
                case 'count-kill':
                    this.host.countKill?.();
                    break;
                case 'count-burial':
                    this.host.countBurial?.();
                    break;
                case 'set-safespot':
                    this.host.setSafespotIndex?.(step.index);
                    break;
                case 'set-target':
                    this.host.targetIdx = step.index == null ? null : step.index;
                    break;
                case 'npc':
                    queue({
                        op: 'npc',
                        name: step.name,
                        action: step.action,
                        index: step.index,
                    });
                    reply = { queued: true };
                    break;
                case 'walk-to':
                    queue({ op: 'walk-to', x: step.x, z: step.z, level: step.level });
                    break;
                case 'sustain':
                    await Sustain.run();
                    break;
                case 'bury':
                    queue({ op: 'held', name: step.name, action: 'Bury' });
                    reply = { buried: true };
                    break;
                case 'delay-ticks':
                    await Execution.delayTicks(Number(step.n) || 1);
                    break;
                case 'wait':
                    await Execution.delayTicks(1);
                    break;
                default:
                    return;
            }
        }
    }

    reset() {
        call({ op: 'reset', token: this.token });
    }

    interruptWatch() {
        call({ op: 'interruptWatch', token: this.token });
    }

    blocksLoot() {
        const out = call({
            op: 'blocksLoot',
            token: this.token,
            ...projection(this.host, this.site),
        });
        return out === true || out?.value === true;
    }
}

export class Retreat {
    constructor(host, site) {
        this.host = host;
        this.site = site;
        const started = retreatCall({ op: 'begin' });
        this.token = started.token;
    }

    validate() {
        const out = retreatCall({ op: 'validate', token: this.token, ...projection(this.host, this.site) });
        return out === true || out?.value === true;
    }

    async execute() {
        this.host.fight?.interruptWatch();
        let reply = null;
        for (;;) {
            const step = retreatCall({
                op: 'next',
                token: this.token,
                reply,
                ...projection(this.host, this.site),
            });
            reply = null;
            if (!step || step.kind === 'yield' || step.kind === 'aborted') {
                return;
            }
            switch (step.kind) {
                case 'log':
                    this.host.log?.(step.message);
                    break;
                case 'status':
                    this.host.setStatus?.(step.message);
                    break;
                case 'set-safespot':
                    this.host.setSafespotIndex?.(step.index);
                    break;
                case 'walk-to':
                    queue({ op: 'walk-to', x: step.x, z: step.z, level: step.level });
                    break;
                case 'sustain':
                    await Sustain.run();
                    break;
                case 'delay-ticks':
                    await Execution.delayTicks(Number(step.n) || 1);
                    break;
                case 'wait':
                    await Execution.delayTicks(1);
                    break;
                default:
                    return;
            }
        }
    }
}

function retreatCall(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_retreat
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('Retreat');
    }
    return fn(payload);
}

function holdCall(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_hold
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('HoldSafespot');
    }
    return fn(payload);
}

export class HoldSafespot {
    constructor(host, site) {
        this.host = host;
        this.site = site;
        const started = holdCall({ op: 'begin' });
        this.token = started.token;
    }

    validate() {
        const out = holdCall({ op: 'validate', token: this.token, ...projection(this.host, this.site) });
        return out === true || out?.value === true;
    }

    async execute() {
        this.host.fight?.interruptWatch();
        let reply = null;
        for (;;) {
            const step = holdCall({
                op: 'next',
                token: this.token,
                reply,
                ...projection(this.host, this.site),
            });
            reply = null;
            if (!step || step.kind === 'yield' || step.kind === 'aborted') {
                return;
            }
            switch (step.kind) {
                case 'log':
                    this.host.log?.(step.message);
                    break;
                case 'status':
                    this.host.setStatus?.(step.message);
                    break;
                case 'walk': {
                    const walkFn = globalThis.rustyscript && globalThis.rustyscript.functions
                        ? globalThis.rustyscript.functions.__rs2b0t_walk
                        : undefined;
                    if (typeof walkFn !== 'function') {
                        throw notImpl('walk');
                    }
                    const walkToken = walkFn({
                        op: 'begin',
                        x: step.x,
                        z: step.z,
                        level: step.level,
                        radius: 0,
                        allow_teleports: false,
                    });
                    queue({
                        op: 'walk',
                        x: step.x,
                        z: step.z,
                        level: step.level,
                        request_id: walkToken,
                        allow_teleports: false,
                        allow_wilderness: true,
                        allow_bank_fetch: true,
                    });
                    reply = { queued: true, walkToken };
                    break;
                }
                case 'sustain':
                    await Sustain.run();
                    break;
                case 'delay-ticks':
                    await Execution.delayTicks(Number(step.n) || 1);
                    break;
                case 'wait':
                    await Execution.delayTicks(1);
                    break;
                case 'set-safespot':
                    this.host.setSafespotIndex?.(step.index);
                    break;
                default:
                    return;
            }
        }
    }
}

function walkspotCall(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_walkspot
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('WalkToSpot');
    }
    return fn(payload);
}

function cellCall(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_cell
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('cell');
    }
    return fn(payload);
}

function cellProjection(site) {
    const item = site.keyItem;
    const out = {
        key: site.key,
        keyItem: item == null ? null : item,
        boxes: site.boxes || [],
    };
    if (site.route) out.route = site.route;
    if (site.outLever) out.outLever = site.outLever;
    if (site.upLadder) out.upLadder = site.upLadder;
    return out;
}

function beginCellWalk(step, radius) {
    const walkFn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_walk
        : undefined;
    if (typeof walkFn !== 'function') {
        throw notImpl('walk');
    }
    return walkFn({
        op: 'begin',
        x: step.x,
        z: step.z,
        level: step.level,
        radius,
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
    });
}

export async function cell(host, site) {
    const started = cellCall({ op: 'begin' });
    if (!started || started.kind === 'aborted' || started.kind === 'notImpl') {
        return false;
    }
    const token = started.token;
    let reply = null;
    for (;;) {
        const step = cellCall({
            op: 'next',
            token,
            reply,
            ...cellProjection(site),
        });
        reply = null;
        if (!step || step.kind === 'aborted' || step.kind === 'notImpl') {
            return false;
        }
        if (step.kind === 'yield') {
            return step.value === true;
        }
        if (step.kind === 'walk-to') {
            queue({ op: 'walk-to', x: step.x, z: step.z, level: step.level });
            reply = { queued: true };
            continue;
        }
        switch (step.kind) {
            case 'log':
                host.log?.(step.message);
                break;
            case 'status':
                host.setStatus?.(step.message);
                break;
            case 'leave':
                reply = { left: (await leaveLair(host, site)) === true };
                break;
            case 'key':
                reply = { held: (await acquireKey(host, site)) === true };
                break;
            case 'walk': {
                const walkToken = beginCellWalk(step, 0);
                queue({
                    op: 'walk',
                    x: step.x,
                    z: step.z,
                    level: step.level,
                    radius: 0,
                    request_id: walkToken,
                    allow_teleports: false,
                    allow_wilderness: false,
                    allow_bank_fetch: false,
                });
                reply = { queued: true, walkToken };
                break;
            }
            case 'walk-near': {
                const radius = Number(step.radius);
                const walkToken = beginCellWalk(step, radius);
                queue({
                    op: 'walk-near',
                    x: step.x,
                    z: step.z,
                    level: step.level,
                    radius,
                    request_id: walkToken,
                    allow_teleports: false,
                    allow_wilderness: false,
                    allow_bank_fetch: false,
                });
                reply = { queued: true, walkToken };
                break;
            }
            case 'npc':
                queue({
                    op: 'npc',
                    name: step.name,
                    action: step.action,
                    index: step.index,
                });
                reply = { queued: true };
                break;
            case 'use-on':
                queue({
                    op: 'use-on',
                    name: step.name,
                    kind: 'loc',
                    x: step.x,
                    z: step.z,
                    level: step.level,
                    source_item_id: step.id,
                    source_item_slot: step.slot,
                    index: null,
                });
                reply = { queued: true };
                break;
            case 'loc':
                queue({
                    op: 'loc',
                    x: step.x,
                    z: step.z,
                    level: step.level,
                    action: step.action,
                    id: step.id,
                });
                reply = { queued: true };
                break;
            case 'continue':
                queue({ op: 'continue' });
                break;
            case 'answer':
                queue({ op: 'answer', option: step.option });
                break;
            case 'sustain':
                await Sustain.run();
                break;
            case 'delay-ticks':
                await Execution.delayTicks(Number(step.n) || 1);
                break;
            case 'wait':
                await Execution.delayTicks(1);
                break;
            default:
                return false;
        }
    }
}

export class WalkToSpot {
    constructor(host, site) {
        this.host = host;
        this.site = site;
        const started = walkspotCall({ op: 'begin' });
        this.token = started.token;
    }

    validate() {
        const out = walkspotCall({ op: 'validate', token: this.token, ...projection(this.host, this.site) });
        return out === true || out?.value === true;
    }

    async execute() {
        this.host.fight?.interruptWatch();
        let reply = null;
        for (;;) {
            const step = walkspotCall({
                op: 'next',
                token: this.token,
                reply,
                ...projection(this.host, this.site),
            });
            reply = null;
            if (!step || step.kind === 'yield' || step.kind === 'aborted') {
                return;
            }
            switch (step.kind) {
                case 'log':
                    this.host.log?.(step.message);
                    break;
                case 'status':
                    this.host.setStatus?.(step.message);
                    break;
                case 'walk': {
                    const walkFn = globalThis.rustyscript && globalThis.rustyscript.functions
                        ? globalThis.rustyscript.functions.__rs2b0t_walk
                        : undefined;
                    if (typeof walkFn !== 'function') {
                        throw notImpl('walk');
                    }
                    const walkToken = walkFn({
                        op: 'begin',
                        x: step.x,
                        z: step.z,
                        level: step.level,
                        radius: 0,
                        allow_teleports: false,
                    });
                    queue({
                        op: 'walk',
                        x: step.x,
                        z: step.z,
                        level: step.level,
                        request_id: walkToken,
                        allow_teleports: false,
                        allow_wilderness: true,
                        allow_bank_fetch: true,
                    });
                    reply = { queued: true, walkToken };
                    break;
                }
                case 'sustain':
                    await Sustain.run();
                    break;
                case 'delay-ticks':
                    await Execution.delayTicks(Number(step.n) || 1);
                    break;
                case 'wait':
                    await Execution.delayTicks(1);
                    break;
                default:
                    return;
            }
        }
    }
}

function enterCall(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_enter
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('EnterLair');
    }
    return fn(payload);
}

function phrase(value) {
    if (!value) return '';
    if (typeof value === 'string') return value;
    if (typeof value.source === 'string') return value.source;
    return String(value);
}

function enterProjection(host, site) {
    const talk = site.talkGate || null;
    const fee = site.feeGate || null;
    const gate = site.gate || null;
    return {
        parked: host.parked === true,
        shieldReady: typeof host.shieldReady === 'function' ? host.shieldReady() === true : false,
        hpFraction: typeof host.hpFraction === 'function' ? host.hpFraction() : 1,
        panicHp: typeof host.panicHp === 'function' ? host.panicHp() : 0.2,
        key: site.key,
        boxes: site.boxes || [],
        approach: (site.approach || []).map(tile),
        talkGate: talk ? {
            npc: talk.npc,
            op: talk.op,
            choose: phrase(talk.choose),
            stand: tile(talk.stand),
        } : null,
        feeGate: fee ? {
            npc: fee.npc,
            op: fee.op,
            coins: fee.coins,
            stand: tile(fee.stand),
            entrance: fee.entrance || null,
            paidLine: phrase(fee.paidLine),
            prepaidLine: phrase(fee.prepaidLine),
        } : null,
        gate: gate ? {
            locId: gate.locId,
            op: gate.op,
            outside: tile(gate.outside),
        } : null,
        keyItem: site.keyItem || null,
        route: site.route || null,
    };
}

function beginWalk(step, radius) {
    const walkFn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_walk
        : undefined;
    if (typeof walkFn !== 'function') {
        throw notImpl('walk');
    }
    return walkFn({
        op: 'begin',
        x: step.x,
        z: step.z,
        level: step.level,
        radius,
        allow_teleports: false,
    });
}

export class EnterLair {
    constructor(host, site) {
        this.host = host;
        this.site = site;
        const started = enterCall({ op: 'begin' });
        this.token = started.token;
    }

    validate() {
        const out = enterCall({ op: 'validate', token: this.token, ...enterProjection(this.host, this.site) });
        return out === true || out?.value === true;
    }

    async execute() {
        let reply = null;
        for (;;) {
            const step = enterCall({
                op: 'next',
                token: this.token,
                reply,
                ...enterProjection(this.host, this.site),
            });
            reply = null;
            if (!step || step.kind === 'aborted') {
                return false;
            }
            if (step.kind === 'yield') {
                return step.value === true;
            }
            switch (step.kind) {
                case 'log':
                    this.host.log?.(step.message);
                    break;
                case 'status':
                    this.host.setStatus?.(step.message);
                    break;
                case 'walk': {
                    const walkToken = beginWalk(step, 0);
                    queue({
                        op: 'walk',
                        x: step.x,
                        z: step.z,
                        level: step.level,
                        radius: 0,
                        request_id: walkToken,
                        allow_teleports: false,
                        allow_wilderness: false,
                        allow_bank_fetch: false,
                    });
                    reply = { queued: true, walkToken };
                    break;
                }
                case 'walk-near': {
                    const walkToken = beginWalk(step, Number(step.radius));
                    queue({
                        op: 'walk-near',
                        x: step.x,
                        z: step.z,
                        level: step.level,
                        radius: Number(step.radius),
                        request_id: walkToken,
                        allow_teleports: false,
                        allow_wilderness: false,
                        allow_bank_fetch: false,
                    });
                    reply = { queued: true, walkToken };
                    break;
                }
                case 'npc':
                    queue({
                        op: 'npc',
                        name: step.name,
                        action: step.action,
                        index: step.index,
                    });
                    reply = { queued: true };
                    break;
                case 'continue':
                    queue({ op: 'continue' });
                    break;
                case 'answer':
                    queue({ op: 'answer', option: step.option });
                    break;
                case 'close-modal':
                    queue({ op: 'close-modal' });
                    break;
                case 'loc':
                    queue({
                        op: 'loc',
                        x: step.x,
                        z: step.z,
                        level: step.level,
                        action: step.action,
                        id: step.id,
                    });
                    reply = { queued: true };
                    break;
                case 'use-on':
                    queue({
                        op: 'use-on',
                        name: step.name,
                        kind: 'loc',
                        x: step.x,
                        z: step.z,
                        level: step.level,
                        source_item_id: step.id,
                        source_item_slot: step.slot,
                        index: null,
                    });
                    reply = { queued: true };
                    break;
                case 'sustain':
                    await Sustain.run();
                    break;
                case 'delay-ticks':
                    await Execution.delayTicks(Number(step.n) || 1);
                    break;
                case 'wait':
                    await Execution.delayTicks(1);
                    break;
                default:
                    return false;
            }
        }
    }
}

function leaveCall(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_leave
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('leaveLair');
    }
    return fn(payload);
}

function leaveProjection(host, site) {
    const exit = site.exit || null;
    const gate = site.gate || null;
    const out = {
        key: site.key,
        boxes: site.boxes || [],
        escapeTeleportId: site.escapeTeleportId ?? null,
        walkOut: tile(site.walkOut),
        exit: exit ? {
            locId: exit.locId,
            op: exit.op,
            stand: tile(exit.stand),
        } : null,
        gate: gate ? {
            locId: gate.locId,
            op: gate.op,
            inside: tile(gate.inside),
        } : null,
    };
    if (typeof host.leaveByWalk === 'function') {
        out.leaveByWalk = host.leaveByWalk() === true;
    }
    if (site.route) out.route = site.route;
    if (site.outLever) out.outLever = site.outLever;
    if (site.upLadder) out.upLadder = site.upLadder;
    return out;
}

function beginLeaveWalk(step, radius) {
    const walkFn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_walk
        : undefined;
    if (typeof walkFn !== 'function') {
        throw notImpl('walk');
    }
    return walkFn({
        op: 'begin',
        x: step.x,
        z: step.z,
        level: step.level,
        radius,
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
    });
}

async function runLeaveTeleport(name) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_teleport
        : undefined;
    if (typeof fn !== 'function') {
        return false;
    }
    let current = fn({ op: 'begin', name: String(name ?? '') });
    if (!current || current.kind === 'unknown' || current.kind === 'notImpl') {
        return false;
    }
    if (current.kind === 'done') {
        return current.result === true;
    }
    const token = current.token;
    while (current && current.kind !== 'done' && current.kind !== 'aborted' && current.kind !== 'unknown' && current.kind !== 'notImpl') {
        if (current.kind === 'if-button') {
            queue({ op: 'if-button', component_id: current.component_id });
        } else if (current.kind !== 'wait') {
            return false;
        }
        let next = null;
        await Execution.delayUntil(() => {
            next = fn({ op: 'next', token });
            return next?.kind !== 'wait';
        }, 0);
        current = next;
    }
    if (current && current.kind === 'done') {
        return current.result === true;
    }
    return false;
}

export async function leaveLair(host, site) {
    const started = leaveCall({ op: 'begin' });
    if (!started || started.kind === 'aborted') {
        return false;
    }
    const token = started.token;
    let reply = null;
    for (;;) {
        const step = leaveCall({
            op: 'next',
            token,
            reply,
            ...leaveProjection(host, site),
        });
        reply = null;
        if (!step || step.kind === 'aborted') {
            return false;
        }
        if (step.kind === 'yield') {
            return step.value === true;
        }
        switch (step.kind) {
            case 'log':
                host.log?.(step.message);
                break;
            case 'status':
                host.setStatus?.(step.message);
                break;
            case 'walk': {
                const walkToken = beginLeaveWalk(step, 0);
                queue({
                    op: 'walk',
                    x: step.x,
                    z: step.z,
                    level: step.level,
                    radius: 0,
                    request_id: walkToken,
                    allow_teleports: false,
                    allow_wilderness: false,
                    allow_bank_fetch: false,
                });
                reply = { queued: true, walkToken };
                break;
            }
            case 'walk-near': {
                const radius = Number(step.radius);
                const walkToken = beginLeaveWalk(step, radius);
                queue({
                    op: 'walk-near',
                    x: step.x,
                    z: step.z,
                    level: step.level,
                    radius,
                    request_id: walkToken,
                    allow_teleports: false,
                    allow_wilderness: false,
                    allow_bank_fetch: false,
                });
                reply = { queued: true, walkToken };
                break;
            }
            case 'teleport':
                reply = { teleported: await runLeaveTeleport(step.name) };
                break;
            case 'loc':
                queue({
                    op: 'loc',
                    x: step.x,
                    z: step.z,
                    level: step.level,
                    action: step.action,
                    id: step.id,
                });
                reply = { queued: true };
                break;
            case 'sustain':
                await Sustain.run();
                break;
            case 'delay-ticks':
                await Execution.delayTicks(Number(step.n) || 1);
                break;
            case 'wait':
                await Execution.delayTicks(1);
                break;
            default:
                return false;
        }
    }
}
function keyCall(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_key
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('acquireKey');
    }
    return fn(payload);
}

function keyProjection(site) {
    const item = site.keyItem;
    const out = {
        key: site.key,
        keyItem: item == null ? null : item,
        boxes: site.boxes || [],
    };
    if (site.route) out.route = site.route;
    if (site.outLever) out.outLever = site.outLever;
    if (site.upLadder) out.upLadder = site.upLadder;
    return out;
}

function beginKeyWalk(step, radius) {
    const walkFn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_walk
        : undefined;
    if (typeof walkFn !== 'function') {
        throw notImpl('walk');
    }
    return walkFn({
        op: 'begin',
        x: step.x,
        z: step.z,
        level: step.level,
        radius,
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
    });
}

export async function acquireKey(host, site) {
    const started = keyCall({ op: 'begin' });
    if (!started || started.kind === 'aborted' || started.kind === 'notImpl') {
        return false;
    }
    const token = started.token;
    let reply = null;
    for (;;) {
        const step = keyCall({
            op: 'next',
            token,
            reply,
            ...keyProjection(site),
        });
        reply = null;
        if (!step || step.kind === 'aborted' || step.kind === 'notImpl') {
            return false;
        }
        if (step.kind === 'yield') {
            return step.value === true;
        }
        switch (step.kind) {
            case 'log':
                host.log?.(step.message);
                break;
            case 'status':
                host.setStatus?.(step.message);
                break;
            case 'leave':
                reply = { left: (await leaveLair(host, site)) === true };
                break;
            case 'walk-near': {
                const radius = Number(step.radius);
                const walkToken = beginKeyWalk(step, radius);
                queue({
                    op: 'walk-near',
                    x: step.x,
                    z: step.z,
                    level: step.level,
                    radius,
                    request_id: walkToken,
                    allow_teleports: false,
                    allow_wilderness: false,
                    allow_bank_fetch: false,
                });
                reply = { queued: true, walkToken };
                break;
            }
            case 'npc':
                queue({
                    op: 'npc',
                    name: step.name,
                    action: step.action,
                    index: step.index,
                });
                reply = { queued: true };
                break;
            case 'obj':
                queue({
                    op: 'obj',
                    x: step.x,
                    z: step.z,
                    level: step.level,
                    name: step.name,
                    action: step.action,
                });
                reply = { queued: true };
                break;
            case 'sustain':
                await Sustain.run();
                break;
            case 'delay-ticks':
                await Execution.delayTicks(Number(step.n) || 1);
                break;
            case 'wait':
                await Execution.delayTicks(1);
                break;
            default:
                return false;
        }
    }
}
