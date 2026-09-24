// PeriodicBank Task: the frozen class surface. Rust decides due (`validate`,
// one call) and runs the bank trip (`execute`, one machine await); the
// caller's options stay script-owned callbacks, called by Rust.
import { runMachine } from '../../shim/_kernel.js';

export class PeriodicBank {
    constructor(opts) {
        this.opts = opts || {};
    }

    validate() {
        return globalThis.__rs2b0t_periodic_bank_validate(this.opts);
    }

    async execute() {
        await runMachine('periodic_bank', {}, this.opts);
    }
}
