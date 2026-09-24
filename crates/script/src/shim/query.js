import { entitySnapView, notImpl, distanceTo } from '../../shim/_kernel.js';

/** Case-insensitive trim compare of posted entity names. */
export function matchesEntityName(actual, configured) {
    if (actual == null || configured == null) {
        return false;
    }
    return String(actual).trim().toLowerCase() === String(configured).trim().toLowerCase();
}

export default class EntityQuery {
    constructor(supplySnaps, wrap) {
        this.supplySnaps = supplySnaps;
        this.wrap = wrap;
        this.snapFilters = [];
        this.entityFilters = [];
    }

    static fromSnapshots(supply, wrap) {
        return new EntityQuery(supply, wrap);
    }

    name(...names) {
        const wanted = names.map((n) => n.trim().toLowerCase());
        this.snapFilters.push(
            (s) => s.name !== null && wanted.includes(String(s.name).trim().toLowerCase()),
        );
        return this;
    }

    action(action) {
        const wanted = String(action).toLowerCase();
        this.snapFilters.push((s) =>
            (s.ops || []).some((a) => a != null && a !== 'hidden' && String(a).toLowerCase() === wanted),
        );
        return this;
    }

    within(dist) {
        this.snapFilters.push((s) => s.distance <= dist);
        return this;
    }

    withinOf(origin, dist) {
        const r = Math.max(0, Math.floor(dist));
        this.snapFilters.push((s) => {
            const t = s.tile;
            return distanceTo(
                { x: origin.x, z: origin.z, level: 0 },
                { x: t.x, z: t.z, level: 0 },
            ) <= r;
        });
        return this;
    }

    where(pred) {
        this.entityFilters.push(pred);
        return this;
    }

    /** Every match in supply order; `seen` true stops the walk. */
    forEachMatch(seen) {
        for (const raw of this.supplySnaps()) {
            const s = entitySnapView(raw);
            if (!s) continue;
            if (this.snapFilters.length > 0 && !this.snapFilters.every((f) => f(s))) continue;
            const e = this.wrap(raw);
            if (this.entityFilters.length > 0 && !this.entityFilters.every((f) => f(e))) continue;
            if (seen(e)) return;
        }
    }

    results() {
        const out = [];
        this.forEachMatch((e) => {
            out.push(e);
            return false;
        });
        return out;
    }

    nearest() {
        let best = null;
        this.forEachMatch((e) => {
            if (!best || e.distance() < best.distance()) best = e;
            return false;
        });
        return best;
    }

    first() {
        let hit = null;
        this.forEachMatch((e) => {
            hit = e;
            return true;
        });
        return hit;
    }

    exists() {
        return this.first() !== null;
    }

    count() {
        let n = 0;
        this.forEachMatch(() => {
            n += 1;
            return false;
        });
        return n;
    }

    inside(_area) {
        throw notImpl('EntityQuery.inside');
    }

    nearestPreferLocal(_preferRadius) {
        throw notImpl('EntityQuery.nearestPreferLocal');
    }
}
