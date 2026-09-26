// Frozen RunManager name map. Policy storage and lifecycle live in Rust.
export const RunManager = {
    override(policy) {
        return globalThis.__rs2b0t_run_override(policy);
    },
};
