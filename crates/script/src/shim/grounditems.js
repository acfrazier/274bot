import Tile from '../../geometry/Tile.js';
import EntityQuery from '../query/Query.js';
import { snap, queue, proxy } from '../../shim/_kernel.js';

export class GroundItem {
    constructor(row) {
        this.snap = row;
    }

    get name() {
        return globalThis.__rs2b0t_entities_v8(51 /* grounditem.name */, this);
    }

    get id() {
        return globalThis.__rs2b0t_entities_v8(52 /* grounditem.id */, this);
    }

    get count() {
        return globalThis.__rs2b0t_entities_v8(53 /* grounditem.count */, this);
    }

    tile() {
        return globalThis.__rs2b0t_entities_v8(54 /* grounditem.tile */, this, Tile);
    }

    distance() {
        return globalThis.__rs2b0t_entities_v8(55 /* grounditem.distance */, this);
    }

    actions() {
        return globalThis.__rs2b0t_entities_v8(56 /* grounditem.actions */, this);
    }

    interact(action) {
        return globalThis.__rs2b0t_entities_v8(57 /* grounditem.interact */, this, action, queue);
    }
}

export const GroundItems = proxy('GroundItems', {
    query() {
        return EntityQuery.fromSnapshots(
            () => snap().ground || [],
            (s) => new GroundItem(s),
        );
    },
});
