// Name-map catalog Input onto Interactions. No MiniMenuAction / client opcodes.
import { snap, queue, proxy, presentOps } from '../shim/_kernel.js';

function npcRow(index) {
    return (snap().npcs || []).find((n) => n && n.index === index) || null;
}

function locRow(x, z) {
    return (snap().locs || []).find((l) => l && l.x === x && l.z === z) || null;
}

function objRow(x, z) {
    return (snap().ground || []).find((o) => o && o.x === x && o.z === z) || null;
}

function invRow(slot) {
    const rs = snap().inv || [];
    if (typeof slot === 'number' && slot >= 0 && slot < rs.length) {
        return rs[slot] || null;
    }
    return rs.find((r) => r && r.slot === slot) || null;
}

function actionAt(row, op) {
    const ops = presentOps(row && row.actions);
    const i = Number(op) - 1;
    return i >= 0 ? ops[i] || null : null;
}

export const Input = proxy('Input', {
    interactNpc(index, op) {
        const row = npcRow(index);
        const action = actionAt(row, op);
        if (!row || !action) return false;
        queue({
            op: 'npc',
            name: row.name ?? '',
            action: String(action),
            index: row.index,
        });
        return true;
    },
    interactPlayer(index, op) {
        const row = (snap().players || []).find((p) => p && p.index === index);
        const action = actionAt(row, op);
        if (!row || !action) return false;
        queue({
            op: 'player',
            name: row.name ?? '',
            action: String(action),
        });
        return true;
    },
    interactLoc(lx, lz, _typecode, op) {
        const row = locRow(lx, lz);
        const action = actionAt(row, op);
        if (!row || !action) return false;
        queue({
            op: 'loc',
            x: row.x,
            z: row.z,
            level: row.level ?? 0,
            action: String(action),
        });
        return true;
    },
    takeObj(lx, lz, _objId, op) {
        const row = objRow(lx, lz);
        const action = actionAt(row, op);
        if (!row || !action) return false;
        queue({
            op: 'obj',
            x: row.x,
            z: row.z,
            level: row.level ?? 0,
            name: row.name ?? null,
            action: String(action),
        });
        return true;
    },
    heldOp(_objId, slot, _comId, op) {
        const row = invRow(slot);
        if (!row) return false;
        const ops = presentOps(row.ops || row.actions);
        const action = ops[Number(op) - 1];
        if (!action) return false;
        queue({ op: 'held', name: row.name ?? '', action: String(action) });
        return true;
    },
});
