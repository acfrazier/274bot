import Tile from '../../geometry/Tile.js';
import EntityQuery from '../query/Query.js';
import { snap, queue, proxy, presentOps, opIndex } from '../../shim/_kernel.js';

export class Player {
    constructor(row) {
        this.snap = row;
    }

    get name() {
        return this.snap.name ?? null;
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
            op: 'player',
            name: this.snap.name ?? '',
            action: String(action),
        });
        return true;
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
        return (snap().players || []).map((s) => new Player(s));
    },
});
