import Tile from '../geometry/Tile.js';

// Ruins tiles are Server runecraft.dbrow exit_coord (level_mx_mz_lx_lz).
function route(rune, talisman, level, bank, x, z) {
    return { rune, talisman, level, bank, ruins: new Tile(x, z, 0) };
}

export const RUNES = {
    'Air rune': route('Air rune', 'Air talisman', 1, 'Falador East', 2983, 3288),
    'Mind rune': route('Mind rune', 'Mind talisman', 2, 'Edgeville', 2980, 3511),
    'Water rune': route('Water rune', 'Water talisman', 5, 'Draynor', 3182, 3162),
    'Earth rune': route('Earth rune', 'Earth talisman', 9, 'Varrock East', 3303, 3477),
    'Fire rune': route('Fire rune', 'Fire talisman', 14, 'Al Kharid', 3310, 3252),
    'Body rune': route('Body rune', 'Body talisman', 20, 'Edgeville', 3050, 3442),
};

export const RUNE_OPTIONS = Object.keys(RUNES);
export const DEFAULT_RUNE = 'Air rune';
