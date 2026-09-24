// DeathRecovery Task: the frozen class surface. Rust latches the death from
// the posted chat (`validate`, one call) and runs the recovery (`execute`,
// one machine await); the caller's hooks stay script-owned callbacks.
// needs/AcquireTask is an explicit unsupported boundary.
import { notImpl, runMachine } from '../../shim/_kernel.js';

export class DeathRecovery {
    constructor(_bot, opts) {
        this.opts = opts || {};
        if (Array.isArray(this.opts.needs) && this.opts.needs.length) {
            throw notImpl('DeathRecovery.needs', 'AcquireTask is unsupported');
        }
    }

    validate() {
        return globalThis.__rs2b0t_death_recovery_validate(this.opts);
    }

    async execute() {
        await runMachine(
            'death_recovery',
            { anchor: this.opts.anchor ?? null, radius: this.opts.radius ?? 6 },
            this.opts,
        );
    }
}
