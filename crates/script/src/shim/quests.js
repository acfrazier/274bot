import { snap, proxy, notImpl } from '../../../shim/_kernel.js';

function rows() {
    return snap().quest_statuses || [];
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
    points() {
        throw notImpl('Quests.points');
    },
});
