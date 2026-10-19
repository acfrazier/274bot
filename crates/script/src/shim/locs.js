import Tile from '../../geometry/Tile.js';
import EntityQuery from '../query/Query.js';
import { queue, proxy } from '../../shim/_kernel.js';

export class Loc {
    constructor(row) {
        this.snap = row;
    }

    get name() {
        return globalThis.__rs2b0t_entities_v8(36 /* loc.name */, this);
    }

    get id() {
        return globalThis.__rs2b0t_entities_v8(37 /* loc.id */, this);
    }

    tile() {
        return globalThis.__rs2b0t_entities_v8(38 /* loc.tile */, this, Tile);
    }

    distance() {
        return globalThis.__rs2b0t_entities_v8(39 /* loc.distance */, this);
    }

    actions() {
        return globalThis.__rs2b0t_entities_v8(40 /* loc.actions */, this);
    }

    interact(action) {
        return globalThis.__rs2b0t_entities_v8(41 /* loc.interact */, this, action, queue);
    }
}

export const Locs = proxy('Locs', {
    query() {
        return globalThis.__rs2b0t_entities_v8(58 /* native query */, 2, EntityQuery, Loc);
    },
});
