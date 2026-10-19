import Tile from '../../geometry/Tile.js';
import EntityQuery from '../query/Query.js';
import { snap, queue, proxy } from '../../shim/_kernel.js';

export class Player {
    constructor(row) {
        this.snap = row;
    }

    get name() {
        return globalThis.__rs2b0t_entities_v8('player.name', this);
    }

    get index() {
        return globalThis.__rs2b0t_entities_v8('player.index', this);
    }

    get inCombat() {
        return globalThis.__rs2b0t_entities_v8('player.inCombat', this);
    }

    get combatLevel() {
        return globalThis.__rs2b0t_entities_v8('player.combatLevel', this);
    }

    /**
     * True when this player's combat target is the local player (`face_entity`),
     * not merely when `Game.inCombat()` is true.
     */
    targetsMe() {
        return globalThis.__rs2b0t_entities_v8('player.targetsMe', this, snap);
    }

    tile() {
        return globalThis.__rs2b0t_entities_v8('player.tile', this, Tile);
    }

    distance() {
        return globalThis.__rs2b0t_entities_v8('player.distance', this);
    }

    actions() {
        return globalThis.__rs2b0t_entities_v8('player.actions', this);
    }

    interact(action) {
        return globalThis.__rs2b0t_entities_v8('player.interact', this, action, queue);
    }
}

export const Players = proxy('Players', {
    query() {
        return EntityQuery.fromSnapshots(
            () => snap().players || [],
            (s) => new Player(s),
        );
    },
    all() {
        return globalThis.__rs2b0t_entities_v8('entities.all', snap, 'players', Player);
    },
});
