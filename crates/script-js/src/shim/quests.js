import { snap, proxy, notImpl } from '../../../shim/_kernel.js';
import { reader } from '../../../adapter/ClientAdapter.js';

function rows() {
    const rs = snap().quest_statuses;
    if (!Array.isArray(rs)) {
        throw notImpl('Quests');
    }
    return rs;
}

export const Quests = proxy('Quests', {
    all() {
        return rows().map((q) => ({
            name: q.name,
            status: q.status || 'unknown',
        }));
    },
    status(name) {
        const want = String(name).toLowerCase();
        const row = rows().find((q) => q && String(q.name).toLowerCase() === want);
        return row ? row.status || 'unknown' : 'unknown';
    },
    journal() {
        throw notImpl('Quests.journal');
    },
    /** Total quest points: transmitted varp `qp` (index 101), as rs2b0t reads it. */
    points() {
        return reader.varp(101);
    },
});
