
/** Case-insensitive trim compare of posted entity names. */
export function matchesEntityName(actual, configured) {
    return globalThis.__rs2b0t_entities_v8('matchesEntityName', actual, configured);
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
        return globalThis.__rs2b0t_entities_v8('query.name', this, names);
    }

    action(action) {
        return globalThis.__rs2b0t_entities_v8('query.action', this, action);
    }

    within(dist) {
        return globalThis.__rs2b0t_entities_v8('query.within', this, dist);
    }

    withinOf(origin, dist) {
        return globalThis.__rs2b0t_entities_v8('query.withinOf', this, origin, dist);
    }

    where(pred) {
        return globalThis.__rs2b0t_entities_v8('query.where', this, pred);
    }

    /** Every match in supply order; `seen` true stops the walk. */
    forEachMatch(seen) {
        return globalThis.__rs2b0t_entities_v8('query.forEachMatch', this, seen);
    }

    results() {
        return globalThis.__rs2b0t_entities_v8('query.results', this);
    }

    nearest() {
        return globalThis.__rs2b0t_entities_v8('query.nearest', this);
    }

    first() {
        return globalThis.__rs2b0t_entities_v8('query.first', this);
    }

    exists() {
        return globalThis.__rs2b0t_entities_v8('query.exists', this);
    }

    count() {
        return globalThis.__rs2b0t_entities_v8('query.count', this);
    }

    inside(_area) {
        return globalThis.__rs2b0t_entities_v8('query.inside', this, _area);
    }

    nearestPreferLocal(_preferRadius) {
        return globalThis.__rs2b0t_entities_v8('query.nearestPreferLocal', this, _preferRadius);
    }
}
