import Tile from '../../geometry/Tile.js';
import EntityQuery from '../query/Query.js';
import { snap, queue, proxy, presentOps, opIndex, notV1 } from '../../shim/_kernel.js';

export class Npc {
    constructor(row) {
        this.snap = row;
    }

    get name() {
        return this.snap.name ?? null;
    }

    get id() {
        return this.snap.id;
    }

    get index() {
        return this.snap.index;
    }

    get inCombat() {
        return this.snap.in_combat === true;
    }

    get health() {
        return this.snap.health;
    }

    get level() {
        throw notV1('Npc.level');
    }

    targetsMe() {
        throw notV1('Npc.targetsMe');
    }

    targetsAnotherPlayer() {
        throw notV1('Npc.targetsAnotherPlayer');
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

    valid() {
        return (snap().npcs || []).some(
            (n) => n && n.index === this.snap.index && n.name === this.snap.name,
        );
    }

    interact(action) {
        const slot = opIndex(this.snap.actions, action);
        if (slot === -1) return false;
        queue({
            op: 'npc',
            name: this.snap.name ?? '',
            action: String(action),
            index: this.snap.index,
        });
        return true;
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
        return (snap().npcs || []).map((s) => new Npc(s));
    },
    nearest(count = 1) {
        return Npcs.all()
            .sort((a, b) => a.distance() - b.distance())
            .slice(0, count);
    },
});

export function talkOp(actions) {
    return (actions || []).find((a) => /^talk/i.test(String(a))) ?? null;
}
