// DeathRecovery Task: ABI matches frozen callers. Rust owns the chat latch,
// Pause/hold freeze, respawn wait and walk sequencing. Callbacks stay script
// owned. needs/AcquireTask is an explicit unsupported boundary.
import { Execution } from '../execution/Execution.js';
import { Game } from '../game/Game.js';
import { chebyshev, notImpl, queue, snap } from '../../shim/_kernel.js';

const WALK_MS = 60000;
const RESPAWN_MS = 20000;
const RESPAWN_TICKS = 3;

function timeoutMs(step, fallback) {
    const n = Number(step && step.timeout_ms);
    return Number.isFinite(n) && n > 0 ? n : fallback;
}

function call(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_death_recovery(payload);
}

function fire(out, opts) {
    if (out && out.fire_on_death) opts.onDeath?.();
    if (out && out.fire_on_recovered) opts.onRecovered?.();
}

function obs(opts) {
    const s = snap();
    return {
        here: s.here || null,
        ingame: s.ingame === true,
        chat_lines: s.chat_lines || [],
        anchor: opts.anchor,
        radius: opts.radius ?? 6,
    };
}

function adjacent(tile, radius) {
    const here = snap().here;
    return here && tile && chebyshev(here, tile) <= radius;
}

export class DeathRecovery {
    constructor(_bot, opts) {
        this.opts = opts || {};
        if (Array.isArray(this.opts.needs) && this.opts.needs.length) {
            throw notImpl('DeathRecovery.needs', 'AcquireTask is unsupported');
        }
    }

    validate() {
        const out = call({ op: 'validate', ...obs(this.opts) });
        fire(out, this.opts);
        return out === true || out?.due === true;
    }

    async execute() {
        if (!this.validate()) {
            return;
        }
        const started = call({
            op: 'begin',
            has_walk_back: typeof this.opts.walkBack === 'function',
            anchor: this.opts.anchor,
            radius: this.opts.radius ?? 6,
        });
        const token = started.token;
        const aborted = () => call({ op: 'current_token' }) !== token;
        const fail = () => {
            if (!aborted()) call({ op: 'fail', token });
        };
        if (started.kind === 'fail' || started.kind === 'aborted') {
            return;
        }
        for (;;) {
            if (aborted()) {
                return;
            }
            const step = call({ op: 'next', token, ...obs(this.opts) });
            switch (step.kind) {
                case 'aborted':
                    return;
                case 'wait':
                    await Execution.delayTicks(1);
                    break;
                case 'wait-respawn': {
                    await Execution.delayUntil(
                        () => aborted() || (Game.ingame() && Game.tile() !== null),
                        timeoutMs(step, RESPAWN_MS),
                    );
                    if (aborted()) return;
                    if (call({ op: 'ack_respawn', token }).kind === 'aborted') return;
                    break;
                }
                case 'wait-ticks': {
                    await Execution.delayTicks(Number(step.ticks) || RESPAWN_TICKS);
                    if (aborted()) return;
                    if (call({ op: 'ack_ticks', token }).kind === 'aborted') return;
                    break;
                }
                case 'walk-near': {
                    const tile = { x: step.x, z: step.z, level: step.level };
                    queue({
                        op: 'walk-near',
                        x: step.x,
                        z: step.z,
                        level: step.level,
                        radius: step.radius,
                        allow_teleports: false,
                    });
                    const ok = await Execution.delayUntil(
                        () => aborted() || adjacent(tile, step.radius),
                        timeoutMs(step, WALK_MS),
                    );
                    if (aborted()) return;
                    if (!ok) {
                        fail();
                        return;
                    }
                    break;
                }
                case 'walk_back': {
                    await this.opts.walkBack?.();
                    if (aborted()) return;
                    const done = call({ op: 'ack_walk_back', token });
                    fire(done, this.opts);
                    return;
                }
                case 'ok':
                    fire(step, this.opts);
                    return;
                case 'fail':
                    fail();
                    return;
                default:
                    fail();
                    return;
            }
        }
    }
}
