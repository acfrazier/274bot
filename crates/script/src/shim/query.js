import { entitySnapView, notV1 } from '../../shim/_kernel.js';

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
            return Math.max(Math.abs(t.x - origin.x), Math.abs(t.z - origin.z)) <= r;
        });
        return this;
    }

    where(pred) {
        this.entityFilters.push(pred);
        return this;
    }

    results() {
        const out = [];
        for (const raw of this.supplySnaps()) {
            const s = entitySnapView(raw);
            if (!s) continue;
            if (this.snapFilters.length > 0 && !this.snapFilters.every((f) => f(s))) continue;
            const e = this.wrap(raw);
            if (this.entityFilters.length > 0 && !this.entityFilters.every((f) => f(e))) continue;
            out.push(e);
        }
        return out;
    }

    nearest() {
        let best = null;
        for (const e of this.results()) {
            if (!best || e.distance() < best.distance()) best = e;
        }
        return best;
    }

    first() {
        return this.results()[0] ?? null;
    }

    exists() {
        return this.results().length > 0;
    }

    count() {
        return this.results().length;
    }

    inside(_area) {
        throw notV1('EntityQuery.inside');
    }

    nearestPreferLocal(_preferRadius) {
        throw notV1('EntityQuery.nearestPreferLocal');
    }
}
