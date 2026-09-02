import Tile from '../../geometry/Tile.js';
import EntityQuery from '../query/Query.js';
import { snap, queue, proxy, presentOps, opIndex } from '../../shim/_kernel.js';

export class GroundItem {
    constructor(row) {
        this.snap = row;
    }

    get name() {
        return this.snap.name ?? null;
    }

    get id() {
        return this.snap.id;
    }

    get count() {
        return typeof this.snap.count === 'number' ? this.snap.count : 1;
    }

    tile() {
        return Tile.from({ x: this.snap.x, z: this.snap.z, level: this.snap.level ?? 0 });
    }

    distance() {
        return typeof this.snap.distance === 'number' ? this.snap.distance : 0;
    }

    actions() {
        return presentOps(this.snap.actions);
    }

    interact(action) {
        if (opIndex(this.snap.actions, action) === -1) return false;
        queue({
            op: 'obj',
            x: this.snap.x,
            z: this.snap.z,
            level: this.snap.level ?? 0,
            name: this.snap.name ?? null,
            action: String(action),
        });
        return true;
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
