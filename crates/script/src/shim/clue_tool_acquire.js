import Tile from '../../../../geometry/Tile.js';
import { notImpl } from '../../../../shim/_kernel.js';

// The ClueSolver card only consumes these immutable catalog names directly.
// Acquisition policy remains in the Rust clue machine.
export const SPADE_NAME = 'Spade';
export const TRIO = Object.freeze(['Sextant', 'Watch', 'Chart']);
export const SPADE_SPAWNS = [new Tile(2574, 3331, 0), new Tile(2981, 3369, 0)];

export const PROFESSOR = {
    npc: 'Observatory professor',
    anchor: new Tile(2438, 3186, 0),
    leash: 10,
    prefer: ['Treasure Trails', 'lost', 'navigation', 'sextant', 'watch'],
};

export const MURPHY = {
    npc: 'Murphy',
    anchor: new Tile(2668, 3162, 0),
    leash: 10,
    prefer: ['sextant', 'lost'],
};

export const KOJO = {
    npc: 'Brother Kojo',
    anchor: new Tile(2569, 3249, 0),
    leash: 10,
    prefer: ['watch', 'lost'],
};

export const KOJO_EXIT = new Tile(2576, 3250, 0);

export function trailKit(scrollId) {
    return globalThis.rustyscript.functions.__rs2b0t_clue({ op: 'trailKit', scrollId });
}

export function nextCoordTool(held) {
    return globalThis.rustyscript.functions.__rs2b0t_clue({ op: 'nextCoordTool', held });
}

export function extraItemShop(name) {
    throw notImpl('extraItemShop');
}

export function gateItemShop(name) {
    throw notImpl('gateItemShop');
}
