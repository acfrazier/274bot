import { proxy, runMachine } from '../../shim/_kernel.js';

const walkApi = globalThis.rustyscript.functions.__rs2b0t_walk_api;

export const WalkExecutor = proxy('WalkExecutor', {
    get lastOutcome() {
        return walkApi('last-outcome');
    },
    async probeDest(dest, maxExpansions) {
        const out = await runMachine('walk-probe-dest', { dest, maxExpansions });
        return out.kind === 'done' ? out.value : { ok: false, terminal: null };
    },
});
