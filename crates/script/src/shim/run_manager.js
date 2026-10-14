// Frozen RunManager name map. Policy storage and lifecycle live in Rust.
import { notImpl } from '../shim/_kernel.js';
export const RunManager = {
    override(policy) {
        return globalThis.__rs2b0t_run_override(policy);
    },
};
export function resolveRunPolicy(over, globals) {
    throw notImpl('resolveRunPolicy');
}

export function shouldEnableRun(s) {
    throw notImpl('shouldEnableRun');
}

