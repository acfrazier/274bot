import Tile from '../geometry/Tile.js';
import { host } from '../shim/_kernel.js';

function route(row) {
    return {
        rune: row.rune,
        talisman: row.talisman,
        level: row.level,
        bank: row.bank,
        ruins: new Tile(row.ruins.x, row.ruins.z, row.ruins.level ?? 0),
    };
}

const rows = (host().content && host().content.rune_routes) || [];
export const RUNES = Object.fromEntries(rows.map((row) => [row.rune, route(row)]));

export const RUNE_OPTIONS = Object.keys(RUNES);
export const DEFAULT_RUNE = 'Air rune';
