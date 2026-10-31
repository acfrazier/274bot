// Scheduler exposes only the active native run context. The host and slot own
// lifecycle transitions; this facade adds no pump or waiter state.
import './ScriptRunner.js';
import { proxy } from '../shim/_kernel.js';

const runtime = (operation) => globalThis.__rs2b0t_script_runtime(operation);

export const Scheduler = proxy('Scheduler', {
    get active() {
        if (!runtime('active')) return null;
        return globalThis.__rs2b0t_script_context_view();
    },
});
