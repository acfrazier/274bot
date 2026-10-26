export const DEFAULT_BOOTH_NAME = globalThis.__rs2b0t_gathering_locations(0);
export const DEFAULT_BOOTH_OP = globalThis.__rs2b0t_gathering_locations(1);
export const MAP_SQUARE = globalThis.__rs2b0t_gathering_locations(2);

export function sameMapSquare(a, b) {
    return globalThis.__rs2b0t_gathering_locations(3, a, b);
}

export function locationOptions(table) {
    return globalThis.__rs2b0t_gathering_locations(4, table);
}

export function boothFields(loc) {
    return globalThis.__rs2b0t_gathering_locations(5, loc);
}

export function resolveGatheringLocation(setting, startTile, table) {
    return globalThis.__rs2b0t_gathering_locations(
        6,
        setting,
        startTile,
        table,
    );
}
