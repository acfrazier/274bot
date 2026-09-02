import Tile from '../../geometry/Tile.js';
import { Game } from '../game/Game.js';

export function nearestBank(_hint) {
    const here = Game.tile();
    if (!here) {
        return null;
    }
    return { tile: new Tile(here.x, here.z, here.level), name: 'Bank booth', op: 'Use-quickly' };
}
