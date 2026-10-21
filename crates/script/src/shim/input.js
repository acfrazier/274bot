// Name-map catalog Input onto Interactions. No MiniMenuAction / client opcodes.
import { snap, queue, proxy, presentOps, notImpl } from '../shim/_kernel.js';

function npcRow(index) {
    return (snap().npcs || []).find((n) => n && n.index === index) || null;
}

function locRow(x, z) {
    return (snap().locs || []).find((l) => l && l.x === x && l.z === z) || null;
}

function actionAt(row, op) {
    const operation = Number(op);
    if (!Number.isInteger(operation) || operation < 1) return null;
    const actions = row && row.actions;
    if (!Array.isArray(actions)) return null;
    const i = operation - 1;
    if (i >= actions.length) return null;
    const action = actions[i];
    if (action == null || action === '') return null;
    if (String(action).toLowerCase() === 'hidden') return null;
    return action;
}

export const Input = proxy('Input', {
    walk(lx, lz) {
        if (!Number.isInteger(lx) || !Number.isInteger(lz)) return false;
        const reach = snap().reach;
        if (
            !reach ||
            reach.available !== true ||
            !Number.isInteger(reach.base_x) ||
            !Number.isInteger(reach.base_z) ||
            !Number.isInteger(reach.level) ||
            !Number.isInteger(reach.width) ||
            !Number.isInteger(reach.height) ||
            lx < 0 ||
            lz < 0 ||
            lx >= reach.width ||
            lz >= reach.height
        ) {
            return false;
        }
        queue({
            op: 'walk-to',
            x: reach.base_x + lx,
            z: reach.base_z + lz,
            level: reach.level,
        });
        return true;
    },
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
    takeObj(lx, lz, objId, op) {
        return globalThis.__rs2b0t_entities_v8(
            59 /* input.takeObj */, lx, lz, Number(objId), Number(op), queue,
        );
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
        // Exact id/slot/component in bank withdraw rows or bank-side deposit rows.
        // Do not fall back by name; Rust revalidates the same identity + session.
        const row =
            (s.bank || []).find(
                (r) =>
                    r &&
                    r.id === id &&
                    r.slot === itemSlot &&
                    r.component_id === component,
            ) ||
            (s.bank_side || []).find(
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
    useItemOnLoc(useObjId, useSlot, useComId, lx, lz, typecode) {
        throw notImpl('Input.useItemOnLoc');
    },
    useItemOnNpc(useObjId, useSlot, useComId, index) {
        throw notImpl('Input.useItemOnNpc');
    },
    useItemOnObj(useObjId, useSlot, useComId, targetObjId, lx, lz) {
        throw notImpl('Input.useItemOnObj');
    },
    useItemOnItem(useObjId, useSlot, useComId, targetObjId, targetSlot, targetComId) {
        throw notImpl('Input.useItemOnItem');
    },
    castOnNpc(spellComId, index) {
        throw notImpl('Input.castOnNpc');
    },
    castOnLoc(spellComId, lx, lz, typecode) {
        throw notImpl('Input.castOnLoc');
    },
    castOnItem(spellComId, itemId, itemSlot, itemComId) {
        throw notImpl('Input.castOnItem');
    },
    continueDialog() {
        throw notImpl('Input.continueDialog');
    },
});
