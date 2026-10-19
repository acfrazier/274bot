
/** Case-insensitive trim compare of posted entity names. */
export function matchesEntityName(actual, configured) {
    return globalThis.__rs2b0t_entities_v8(3 /* matchesEntityName */, actual, configured);
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
        return globalThis.__rs2b0t_entities_v8(7 /* query.name */, this, names);
    }

    action(action) {
        return globalThis.__rs2b0t_entities_v8(8 /* query.action */, this, action);
    }

    within(dist) {
        return globalThis.__rs2b0t_entities_v8(9 /* query.within */, this, dist);
    }

    withinOf(origin, dist) {
        return globalThis.__rs2b0t_entities_v8(10 /* query.withinOf */, this, origin, dist);
    }

    where(pred) {
        return globalThis.__rs2b0t_entities_v8(12 /* query.where */, this, pred);
    }

    /** Every match in supply order; `seen` true stops the walk. */
    forEachMatch(seen) {
        return globalThis.__rs2b0t_entities_v8(13 /* query.forEachMatch */, this, seen);
    }

    results() {
        return globalThis.__rs2b0t_entities_v8(14 /* query.results */, this);
    }

    nearest() {
        return globalThis.__rs2b0t_entities_v8(15 /* query.nearest */, this);
    }

    first() {
        return globalThis.__rs2b0t_entities_v8(16 /* query.first */, this);
    }

    exists() {
        return globalThis.__rs2b0t_entities_v8(17 /* query.exists */, this);
    }

    count() {
        return globalThis.__rs2b0t_entities_v8(18 /* query.count */, this);
    }

    inside(_area) {
        return globalThis.__rs2b0t_entities_v8(11 /* query.inside */, this, _area);
    }

    nearestPreferLocal(_preferRadius) {
        return globalThis.__rs2b0t_entities_v8(19 /* query.nearestPreferLocal */, this, _preferRadius);
    }
}
