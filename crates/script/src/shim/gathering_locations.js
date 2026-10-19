export const DEFAULT_BOOTH_NAME = globalThis.__rs2b0t_gathering_locations('DEFAULT_BOOTH_NAME');
export const DEFAULT_BOOTH_OP = globalThis.__rs2b0t_gathering_locations('DEFAULT_BOOTH_OP');
export const MAP_SQUARE = globalThis.__rs2b0t_gathering_locations('MAP_SQUARE');

export function sameMapSquare(a, b) {
    return globalThis.__rs2b0t_gathering_locations('sameMapSquare', a, b);
}

export function locationOptions(table) {
    return globalThis.__rs2b0t_gathering_locations('locationOptions', table);
}

export function boothFields(loc) {
    return globalThis.__rs2b0t_gathering_locations('boothFields', loc);
}

export function resolveGatheringLocation(setting, startTile, table) {
    return globalThis.__rs2b0t_gathering_locations(
        'resolveGatheringLocation',
        setting,
        startTile,
        table,
    );
}
