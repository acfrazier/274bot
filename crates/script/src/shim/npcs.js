import Tile from '../../geometry/Tile.js';
import EntityQuery from '../query/Query.js';
import { queue, proxy } from '../../shim/_kernel.js';

export class Npc {
    constructor(row) {
        this.snap = row;
    }

    get name() {
        return globalThis.__rs2b0t_entities_v8(20 /* npc.name */, this);
    }

    get id() {
        return globalThis.__rs2b0t_entities_v8(21 /* npc.id */, this);
    }

    get index() {
        return globalThis.__rs2b0t_entities_v8(22 /* npc.index */, this);
    }

    get inCombat() {
        return globalThis.__rs2b0t_entities_v8(23 /* npc.inCombat */, this);
    }

    get health() {
        return globalThis.__rs2b0t_entities_v8(24 /* npc.health */, this);
    }

    get level() {
        return globalThis.__rs2b0t_entities_v8(25 /* npc.level */, this);
    }

    get size() {
        return globalThis.__rs2b0t_entities_v8(26 /* npc.size */, this);
    }

    networkOrigin() {
        return globalThis.__rs2b0t_entities_v8(27 /* npc.networkOrigin */, this, Tile);
    }

    networkTile() {
        // Frozen Npc.ts:55-57 maps a missing network tile to tile().
        return globalThis.__rs2b0t_entities_v8(28 /* npc.networkTile */, this, Tile);
    }

    /**
     * True when this NPC's combat target is the local player (`face_entity`),
     * not merely when `Game.inCombat()` is true.
     */
    targetsMe() {
        return globalThis.__rs2b0t_entities_v8(29 /* npc.targetsMe */, this);
    }

    targetsAnotherPlayer() {
        return globalThis.__rs2b0t_entities_v8(30 /* npc.targetsAnotherPlayer */, this);
    }

    tile() {
        return globalThis.__rs2b0t_entities_v8(31 /* npc.tile */, this, Tile);
    }

    distance() {
        return globalThis.__rs2b0t_entities_v8(32 /* npc.distance */, this);
    }

    actions() {
        return globalThis.__rs2b0t_entities_v8(33 /* npc.actions */, this);
    }

    valid() {
        return globalThis.__rs2b0t_entities_v8(34 /* npc.valid */, this);
    }

    interact(action) {
        return globalThis.__rs2b0t_entities_v8(35 /* npc.interact */, this, action, queue);
    }
}

export const Npcs = proxy('Npcs', {
    query() {
        return globalThis.__rs2b0t_entities_v8(58 /* native query */, 0, EntityQuery, Npc);
    },
    all() {
        return globalThis.__rs2b0t_entities_v8(5 /* entities.all */, 0, Npc);
    },
    nearest(count = 1) {
        return globalThis.__rs2b0t_entities_v8(6 /* npcs.nearest */, Npcs, count);
    },
});

export function talkOp(actions) {
    return globalThis.__rs2b0t_entities_v8(4 /* talkOp */, actions);
}
