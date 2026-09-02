import Tile from '../../geometry/Tile.js';
import EntityQuery from '../query/Query.js';
import { snap, queue, proxy, presentOps, opIndex } from '../../shim/_kernel.js';

export class Loc {
    constructor(row) {
        this.snap = row;
    }

    get name() {
        return this.snap.name ?? null;
    }

    get id() {
        return this.snap.id;
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
            op: 'loc',
            x: this.snap.x,
            z: this.snap.z,
            level: this.snap.level ?? 0,
            action: String(action),
        });
        return true;
    }
}

export const Locs = proxy('Locs', {
    query() {
        return EntityQuery.fromSnapshots(
            () => snap().locs || [],
            (s) => new Loc(s),
        );
    },
});
