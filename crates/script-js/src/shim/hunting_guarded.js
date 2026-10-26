// Loot guard: a name map onto the Rust gap rule (`crate::hunt_catalog`).
export const LOOT_GUARD = 4;

export function guarded(drop, bodies, radius = LOOT_GUARD) {
    return globalThis.__rs2b0t_hunt_logic('guarded', drop, bodies, radius);
}
