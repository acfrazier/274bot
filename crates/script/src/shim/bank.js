import { Execution } from '../execution/Execution.js';
import { Inventory } from '../inventory/Inventory.js';
import { runMachine } from '../../shim/_kernel.js';
const host = () => globalThis.__rs2b0t_host || {};
const notImpl = (name, reason) =>
    new Error(reason ? 'not impl: ' + name + ': ' + reason : 'not impl: ' + name);
const snap = () => host().snapshot || {};
const queue = (req) => {
    const h = host();
    h.interact = h.interact || [];
    h.interact.push(req);
};

// One try: Rust walks, presses the booth it observed and waits for the
// fresh item list, then the caller's await answers the frozen boolean.
async function driveBankOpen(input) {
    const out = await runMachine('bank_open', {
        mode: input.mode,
        stand: input.stand ?? null,
        booth_name: input.booth_name ?? null,
        booth_action: input.booth_action ?? null,
    });
    return out.kind === 'done' && out.value === true;
}

// One bank item op: Rust decides the action from the posted rows, sends it
// and awaits the host result. Arguments Rust cannot read are refused.
async function bankOp(args) {
    const out = await runMachine('bank_op', args);
    if (out.kind === 'refused') throw notImpl(out.reason);
    return out.kind === 'done' && out.value === true;
}

// One Rust deposit loop (frozen depositAllMatching).
async function deposit(args, hooks = {}) {
    const out = await runMachine('bank_deposit', args, hooks);
    if (out.kind === 'refused') throw notImpl('Bank.depositAllMatching', out.reason);
}

// Frozen `withdrawOp`: the row's own op matching the amount, in Rust.
export function withdrawOp(ops, which) {
    return globalThis.__rs2b0t_withdraw_op(ops, which);
}

export const Bank = new Proxy(
    {
        isOpen() {
            return snap().bank_open === true;
        },
        loaded() {
            return (snap().bank || []).length > 0;
        },
        ready() {
            return Bank.isOpen() && (Bank.snapshotReady() || Bank.loaded());
        },
        items() {
            return (snap().bank || []).map((row) => ({
                name: row?.name ?? null,
                count: row?.count ?? 0,
                id: row?.id ?? 0,
                slot: typeof row?.slot === 'number' ? row.slot : -1,
                comId: typeof row?.component_id === 'number' ? row.component_id : -1,
                ops: Array.isArray(row?.ops) ? row.ops.slice() : [],
                noted: row?.noted === true,
                cert: row?.cert ?? -1,
            }));
        },
        count(name) {
            const wanted = String(name).toLowerCase();
            return (snap().bank || [])
                .filter(
                    (row) =>
                        row && typeof row.name === 'string' && row.name.toLowerCase() === wanted,
                )
                .reduce((sum, row) => sum + (typeof row.count === 'number' ? row.count : 0), 0);
        },
        deposit(name) {
            return bankOp({ kind: 'deposit', name: String(name) });
        },
        async depositInventory() {
            await deposit({ all: true });
        },
        async depositAllMatching(predicate, log) {
            await deposit(
                {},
                {
                    match: typeof predicate === 'function' ? predicate : undefined,
                    log: typeof log === 'function' ? log : undefined,
                },
            );
        },
        async depositAllExcept(keep) {
            await deposit({ keep: Array.from(keep || []).map((k) => String(k)) });
        },
        withdraw(name, amount) {
            return bankOp({ kind: 'withdraw', name: String(name), amount });
        },
        async setNoteMode(on) {
            if (!Bank.isOpen()) {
                throw notImpl('Bank.setNoteMode');
            }
            const wantOn = !!on;
            const btnId = wantOn ? snap().bank_note_on ?? -1 : snap().bank_note_off ?? -1;
            if (typeof btnId !== 'number' || btnId < 0) {
                throw notImpl('Bank.setNoteMode');
            }
            queue({ op: 'set-note-mode', on: wantOn });
            await Execution.delayTicks(1);
            return true;
        },
        async close() {
            if (!Bank.isOpen()) {
                return true;
            }
            queue({ op: 'close' });
            return Execution.delayUntil(() => !Bank.isOpen(), 3000);
        },
        withdrawById(id, op = 'Withdraw-1') {
            return bankOp({ kind: 'withdraw-id', id, op: String(op) });
        },
        withdrawX(name, count) {
            return bankOp({ kind: 'withdraw-x', name: String(name), count });
        },
        withdrawXById(id, count, landsAsId = id) {
            return bankOp({ kind: 'withdraw-x-id', id, count, lands_as: landsAsId });
        },
        async openBooth(stand, boothName, op, _log) {
            const named = boothName !== undefined || op !== undefined;
            return driveBankOpen({
                mode: 'open-booth',
                stand: stand ?? null,
                booth_name: named ? String(boothName ?? 'Bank booth') : null,
                booth_action: named ? String(op ?? 'Use-quickly') : null,
            });
        },
        async openNearest(boothName, op, log) {
            const named = boothName !== undefined || op !== undefined;
            if (!named) {
                return Bank.openBooth(undefined, boothName, op, log);
            }
            return driveBankOpen({
                mode: 'open-nearest',
                stand: null,
                booth_name: String(boothName ?? 'Bank booth'),
                booth_action: String(op ?? 'Use-quickly'),
            });
        },
        async openNearestWorld() {
            return driveBankOpen({
                mode: 'open-nearest-world',
                stand: null,
                booth_name: null,
                booth_action: null,
            });
        },
        async waitReady(timeoutMs, log) {
            const ms = typeof timeoutMs === 'number' && timeoutMs > 0 ? timeoutMs : 5000;
            if (!Bank.isOpen() || Bank.ready()) {
                return Bank.ready();
            }
            await Execution.delayUntil(() => !Bank.isOpen() || Bank.ready(), ms);
            if (Bank.isOpen() && !Bank.ready() && typeof log === 'function') {
                log('bank: opened but the item list never arrived');
            }
            return Bank.ready();
        },
        snapshotReady() {
            return snap().bank_loaded === true;
        },
        snapshotGeneration() {
            const generation = snap().bank_generation;
            return Number.isFinite(generation) && generation >= 0 ? generation : 0;
        },
        async waitSnapshotAfter(generation, timeoutMs) {
            const baseline = Number(generation);
            if (!Number.isFinite(baseline) || baseline < 0) {
                return false;
            }
            const ms = typeof timeoutMs === 'number' && timeoutMs > 0 ? timeoutMs : 5000;
            await Execution.delayUntil(
                () => !Bank.isOpen() || Bank.snapshotGeneration() > baseline,
                ms,
            );
            return (
                Bank.isOpen() &&
                Bank.snapshotReady() &&
                Bank.snapshotGeneration() > baseline
            );
        },
        countById(id) {
            const rs = snap().bank || [];
            if (rs.length === 0) {
                return 0;
            }
            if (!rs.some((r) => r && typeof r.id === 'number' && r.id !== 0)) {
                throw notImpl('Bank.countById');
            }
            const want = Number(id);
            return rs
                .filter((r) => r && r.id === want)
                .reduce((sum, row) => sum + (typeof row.count === 'number' ? row.count : 0), 0);
        },
        async withdrawLoad(name) {
            if (!Bank.ready()) return false;
            if (Inventory.isFull()) return true;
            const wanted = String(name).toLowerCase();
            const row = (snap().bank || []).find(
                (r) =>
                    r &&
                    typeof r.name === 'string' &&
                    r.name.toLowerCase() === wanted &&
                    Number(r.count) > 0,
            );
            if (!row) return false;
            const generation = Bank.snapshotGeneration();
            const resultSeq = Number(snap().withdraw_load_result_seq) || 0;
            queue({ op: 'withdraw-load', name: row.name, bank_generation: generation });
            await Execution.delayUntil(
                () =>
                    (Number(snap().withdraw_load_result_seq) || 0) !== resultSeq ||
                    !Bank.isOpen() ||
                    Bank.snapshotGeneration() !== generation,
                0,
            );
            return (
                Bank.isOpen() &&
                Bank.snapshotGeneration() === generation &&
                (Number(snap().withdraw_load_result_seq) || 0) !== resultSeq &&
                snap().withdraw_load_result === true
            );
        },
        async openNearestAccess(access, log) {
            const first = access?.openFirst;
            const out = await runMachine(
                'bank_access',
                {
                    name: String(access?.name ?? 'Bank booth'),
                    op: String(access?.op ?? 'Use-quickly'),
                    open_first: first ? { name: String(first.name), op: String(first.op) } : null,
                },
                { log: typeof log === 'function' ? log : undefined },
            );
            return out.kind === 'done' && out.value === true;
        },
        async openNpcAccess(access, log) {
            const out = await runMachine(
                'bank_npc_access',
                { name: String(access.name), op: String(access.op), choose: String(access.choose ?? '') },
                { log: typeof log === 'function' ? log : undefined },
            );
            return out.kind === 'done' && out.value === true;
        },
    },
    {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notImpl('Bank.' + String(prop));
        },
    },
);
