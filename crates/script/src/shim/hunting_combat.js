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
    constructor() {
        throw notImpl('Retreat');
    }
}

export class HoldSafespot {
    constructor() {
        throw notImpl('HoldSafespot');
    }
}

export class WalkToSpot {
    constructor() {
        throw notImpl('WalkToSpot');
    }
}
