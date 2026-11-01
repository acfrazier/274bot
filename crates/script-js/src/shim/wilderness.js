export function wildernessLevelAt(tile) {
    const { x, z } = tile;
    const level = tile.level ?? 0;
    return globalThis.rustyscript.functions.__rs2b0t_walk_api(
        'wilderness-level-at',
        { x, z, level },
    );
}
