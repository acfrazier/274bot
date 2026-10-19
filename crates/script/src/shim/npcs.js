import Tile from '../../geometry/Tile.js';
import EntityQuery from '../query/Query.js';
import { snap, queue, proxy } from '../../shim/_kernel.js';

export class Npc {
    constructor(row) {
        this.snap = row;
    }

    get name() {
        return globalThis.__rs2b0t_entities_v8('npc.name', this);
    }

    get id() {
        return globalThis.__rs2b0t_entities_v8('npc.id', this);
    }

    get index() {
        return globalThis.__rs2b0t_entities_v8('npc.index', this);
    }

    get inCombat() {
        return globalThis.__rs2b0t_entities_v8('npc.inCombat', this);
    }

    get health() {
        return globalThis.__rs2b0t_entities_v8('npc.health', this);
    }

    get level() {
        return globalThis.__rs2b0t_entities_v8('npc.level', this);
    }

    get size() {
        return globalThis.__rs2b0t_entities_v8('npc.size', this);
    }

    networkOrigin() {
        return globalThis.__rs2b0t_entities_v8('npc.networkOrigin', this, Tile);
    }

    networkTile() {
        // Frozen Npc.ts:55-57 maps a missing network tile to tile().
        return globalThis.__rs2b0t_entities_v8('npc.networkTile', this, Tile);
    }

    /**
     * True when this NPC's combat target is the local player (`face_entity`),
     * not merely when `Game.inCombat()` is true.
     */
    targetsMe() {
        return globalThis.__rs2b0t_entities_v8('npc.targetsMe', this, snap);
    }

    targetsAnotherPlayer() {
        return globalThis.__rs2b0t_entities_v8('npc.targetsAnotherPlayer', this, snap);
    }

    tile() {
        return globalThis.__rs2b0t_entities_v8('npc.tile', this, Tile);
    }

    distance() {
        return globalThis.__rs2b0t_entities_v8('npc.distance', this);
    }

    actions() {
        return globalThis.__rs2b0t_entities_v8('npc.actions', this);
    }

    valid() {
        return globalThis.__rs2b0t_entities_v8('npc.valid', this, snap);
    }

    interact(action) {
        return globalThis.__rs2b0t_entities_v8('npc.interact', this, action, queue);
    }
}

export const Npcs = proxy('Npcs', {
    query() {
        return EntityQuery.fromSnapshots(
            () => snap().npcs || [],
            (s) => new Npc(s),
        );
    },
    all() {
        return globalThis.__rs2b0t_entities_v8('entities.all', snap, 'npcs', Npc);
    },
    nearest(count = 1) {
        return globalThis.__rs2b0t_entities_v8('npcs.nearest', Npcs, count);
    },
});

export function talkOp(actions) {
    return globalThis.__rs2b0t_entities_v8('talkOp', actions);
}
