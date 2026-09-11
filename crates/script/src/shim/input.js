// Name-map catalog Input onto Interactions. No MiniMenuAction / client opcodes.
import { snap, queue, proxy, presentOps, notImpl } from '../shim/_kernel.js';

function npcRow(index) {
    return (snap().npcs || []).find((n) => n && n.index === index) || null;
}

function locRow(x, z) {
    return (snap().locs || []).find((l) => l && l.x === x && l.z === z) || null;
}

function objRow(x, z) {
    return (snap().ground || []).find((o) => o && o.x === x && o.z === z) || null;
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
    heldOp(objId, _slot, _comId, op) {
        const rs = snap().inv || [];
        const hasId = rs.some((r) => r && typeof r.id === 'number' && r.id !== 0);
        if (!hasId) throw notImpl('Input.heldOp');
        const row = rs.find((r) => r && r.id === objId) || null;
        if (!row) return false;
        const ops = presentOps(row.ops || row.actions);
        const action = ops[Number(op) - 1];
        if (!action) return false;
        queue({ op: 'held', name: row.name ?? '', action: String(action) });
        return true;
    },
    invButton(objId, slot, comId, op) {
        const id = Number(objId);
        const itemSlot = Number(slot);
        const component = Number(comId);
        const operation = Number(op);
        if (
            ![id, itemSlot, component, operation].every((n) => Number.isInteger(n)) ||
            itemSlot < 0 ||
            component < 0 ||
            operation < 1
        ) {
            return false;
        }
        const s = snap();
        if (s.bank_open !== true || s.bank_loaded !== true) return false;
        const row = (s.bank || []).find(
            (r) =>
                r &&
                r.id === id &&
                r.slot === itemSlot &&
                r.component_id === component,
        );
        if (!row) return false;
        const ops = Array.isArray(row.ops) ? row.ops : [];
        const action = ops[operation - 1];
        if (!action || String(action).toLowerCase() === 'hidden') return false;
        const generation = Number(s.bank_generation);
        queue({
            op: 'inv-button',
            id,
            slot: itemSlot,
            component,
            operation,
            bank_generation:
                Number.isFinite(generation) && generation >= 0 ? generation : 0,
        });
        return true;
    },
});
