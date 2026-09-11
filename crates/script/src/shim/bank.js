import { Execution } from '../execution/Execution.js';
import { Inventory } from '../inventory/Inventory.js';
const host = () => globalThis.__rs2b0t_host || {};
const notImpl = (name, reason) =>
    new Error(reason ? 'not impl: ' + name + ': ' + reason : 'not impl: ' + name);
const snap = () => host().snapshot || {};
const queue = (req) => {
    const h = host();
    h.interact = h.interact || [];
    h.interact.push(req);
};
let withdrawXPending = false;
let bankOpPending = false;

function bankOpenCall(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_bank_open(payload);
}

async function driveBankOpen(input) {
    let step = bankOpenCall({ op: 'begin', ...input });
    const token = step?.token;
    while (step && step.kind !== 'done' && step.kind !== 'aborted') {
        if (step.kind === 'walk-near') {
            queue({
                op: 'walk-near',
                x: step.x,
                z: step.z,
                level: step.level,
                radius: step.radius,
                allow_teleports: step.allow_teleports === true,
            });
        } else if (step.kind === 'walk-nearest-bank') {
            queue({ op: 'walk-nearest-bank' });
        } else if (step.kind === 'open-booth') {
            queue({
                op: 'open-booth',
                x: step.x,
                z: step.z,
                level: step.level,
                id: step.id,
                ...(typeof step.name === 'string' ? { name: step.name } : {}),
                ...(typeof step.action === 'string' ? { action: step.action } : {}),
            });
        } else if (step.kind !== 'wait') {
            return false;
        }
        let next = null;
        await Execution.delayUntil(() => {
            next = bankOpenCall({
                op: 'next',
                token,
            });
            return next?.kind !== 'wait';
        }, 0);
        step = next;
    }
    return step?.kind === 'done' && step.ok === true;
}

function bankOp(req) {
    if (!Bank.ready() || bankOpPending) return Promise.resolve(false);
    const generation = Bank.snapshotGeneration();
    const resultSeq = Number(snap().bank_op_result_seq) || 0;
    bankOpPending = true;
    queue(req);
    return (async () => {
        try {
            await Execution.delayUntil(
                () =>
                    (Number(snap().bank_op_result_seq) || 0) !== resultSeq ||
                    !Bank.isOpen() ||
                    Bank.snapshotGeneration() !== generation,
                0,
            );
            return (
                Bank.isOpen() &&
                Bank.snapshotGeneration() === generation &&
                (Number(snap().bank_op_result_seq) || 0) !== resultSeq &&
                snap().bank_op_result === true
            );
        } finally {
            bankOpPending = false;
        }
    })();
}

function withdrawXRow(row, count, landsAsId) {
    const amount = Number(count);
    if (
        !Bank.ready() ||
        !Number.isSafeInteger(amount) ||
        amount <= 0 ||
        amount > 2147483647 ||
        !Number.isSafeInteger(row?.id) ||
        !Number.isSafeInteger(landsAsId) ||
        landsAsId < -2147483648 ||
        landsAsId > 2147483647 ||
        snap().count_dialog_open ||
        withdrawXPending
    ) {
        return Promise.resolve(false);
    }
    const available = Math.max(0, Number(row.count) || 0);
    if (available === 0) return Promise.resolve(false);
    const take = Math.min(amount, available);
    const ops = Array.isArray(row.ops) ? row.ops : [];
    const fixed = [1, 5, 10].includes(take)
        ? ops.find(
              (action) =>
                  action &&
                  new RegExp(`^withdraw[\\s-]*${take}$`, 'i').test(String(action).trim()),
          )
        : null;
    const action =
        fixed ||
        ops.find(
            (candidate) =>
                candidate &&
                String(candidate).replace(/-/g, ' ').trim().toLowerCase() === 'withdraw x',
        );
    if (!action) throw notImpl('Bank.withdrawX');
    const generation = Bank.snapshotGeneration();
    const resultSeq = Number(snap().withdraw_x_result_seq) || 0;
    withdrawXPending = true;
    queue({
        op: 'withdraw-x',
        name: row.name,
        count: take,
        bank_item_id: row.id,
        lands_as_id: landsAsId,
        action,
        bank_generation: generation,
    });
    return (async () => {
        try {
            // Rust owns the bounded dialog/settlement deadline and always posts
            // an outcome, including rejection and session abort. Do not race it
            // with an isolate wall clock that keeps advancing during Pause/hold.
            await Execution.delayUntil(
                () =>
                    (Number(snap().withdraw_x_result_seq) || 0) !== resultSeq ||
                    !Bank.isOpen() ||
                    Bank.snapshotGeneration() !== generation,
                0,
            );
            return (
                Bank.isOpen() &&
                Bank.snapshotGeneration() === generation &&
                (Number(snap().withdraw_x_result_seq) || 0) !== resultSeq &&
                snap().withdraw_x_result === true
            );
        } finally {
            withdrawXPending = false;
        }
    })();
}

// Pick the withdraw op for a requested amount (rs2b0t `withdrawOp`).
// The posted bank rows carry no op labels; the shim maps the requested
// amount straight to the `Withdraw …` action label the host dispatches.
export function withdrawOp(ops, which) {
    const labels = { all: 'Withdraw All', '10': 'Withdraw 10', '1': 'Withdraw 1' };
    return labels[String(which)] || null;
}

export const Bank = new Proxy(
    {
        isOpen() {
            return snap().bank_open === true;
        },
        // The withdraw list has actually been decoded (a bank whose list
        // has not filled reads empty, which is not proof of an empty bank).
        loaded() {
            return snap().bank_loaded === true;
        },
        ready() {
            return Bank.isOpen() && Bank.loaded();
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
        // Deposit-all the named bank-side item. The deposit op is always
        // the row's "all" op (`Deposit-1/5/10/All`), the only op worth
        // sending here; the specific-op arm is not impl.
        deposit(name) {
            const wanted = String(name).toLowerCase();
            for (const row of snap().bank_side || []) {
                if (row && typeof row.name === 'string' && row.name.toLowerCase() === wanted) {
                    return bankOp({ op: 'deposit', name: row.name });
                }
            }
            return Promise.resolve(false);
        },
        async depositInventory() {
            await Bank.depositAllMatching(() => true);
        },
        async depositAllMatching(predicate) {
            if (typeof predicate !== 'function') {
                throw notImpl('Bank.depositAllMatching', 'requires a function');
            }
            for (let guard = 0; guard < 32; guard++) {
                let rows = snap().bank_side || [];
                if (rows.length === 0 && Bank.isOpen()) {
                    await Execution.delayUntil(
                        () => (snap().bank_side || []).length > 0 || !Bank.isOpen(),
                        1200,
                    );
                    rows = snap().bank_side || [];
                }
                const row = rows.find(
                    (item) =>
                        item &&
                        predicate(item.name ?? '', item.id ?? -1),
                );
                if (!row) return;
                if (typeof row.name !== 'string') return;
                if (!(await bankOp({ op: 'deposit', name: row.name }))) return;
            }
        },
        async depositAllExcept(keep) {
            const kept = new Set(Array.from(keep || []).map((k) => String(k).toLowerCase()));
            await Bank.depositAllMatching((name) => !kept.has(name.toLowerCase()));
        },
        // Withdraw by name + op: an action label string is used verbatim
        // (`Withdraw All` / `Withdraw 10` / `Withdraw 1`, or `'all'` for
        // Withdraw All); a number maps to the brief's op set — all when it
        // would cover the row's whole count (and is 10+), else 10, else 1.
        withdraw(name, amount) {
            const wanted = String(name).toLowerCase();
            let action;
            if (typeof amount === 'string') {
                action = amount.toLowerCase() === 'all' ? 'Withdraw All' : amount;
            } else {
                const n = Number(amount);
                const row = (snap().bank || []).find(
                    (r) => r && typeof r.name === 'string' && r.name.toLowerCase() === wanted,
                );
                const count = row && typeof row.count === 'number' ? row.count : 0;
                action = n >= 10 && n >= count ? 'Withdraw All' : n >= 10 ? 'Withdraw 10' : 'Withdraw 1';
            }
            return bankOp({ op: 'withdraw', name: String(name), action });
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
        async withdrawById(id, op = 'Withdraw-1') {
            const row = (snap().bank || []).find((r) => r && r.id === id);
            if (!row?.name) {
                return false;
            }
            const action =
                op.toLowerCase() === 'withdraw-all' || op.toLowerCase() === 'all'
                    ? 'Withdraw All'
                    : op.replace(/-/g, ' ');
            return bankOp({ op: 'withdraw', name: row.name, action });
        },
        withdrawX(name, count) {
            const amount = Number(count);
            if (Number.isSafeInteger(amount) && amount <= 0) {
                return Promise.resolve(true);
            }
            const wanted = String(name).toLowerCase();
            const row = (snap().bank || []).find(
                (r) => r && typeof r.name === 'string' && r.name.toLowerCase() === wanted,
            );
            if (!row) {
                throw notImpl('Bank.withdrawX');
            }
            return withdrawXRow(row, amount, row.id);
        },
        async withdrawXById(id, count, landsAsId = id) {
            const amount = Number(count);
            if (Number.isSafeInteger(amount) && amount <= 0) {
                return true;
            }
            const row = (snap().bank || []).find((r) => r && r.id === id);
            if (!row?.name) {
                return false;
            }
            return withdrawXRow(row, amount, Number(landsAsId));
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
        async waitReady(timeoutMs, _log) {
            const ms = typeof timeoutMs === 'number' && timeoutMs > 0 ? timeoutMs : 5000;
            return Execution.delayUntil(() => Bank.ready(), ms);
        },
        snapshotReady() {
            return Bank.ready();
        },
        snapshotGeneration() {
            const generation = snap().bank_generation;
            return Number.isFinite(generation) && generation >= 0 ? generation : 0;
        },
        async waitSnapshotAfter(generation, timeoutMs) {
            const baseline = Number.isFinite(Number(generation)) ? Number(generation) : 0;
            const ms = typeof timeoutMs === 'number' && timeoutMs > 0 ? timeoutMs : 5000;
            return Execution.delayUntil(
                () => Bank.snapshotReady() && Bank.snapshotGeneration() > baseline,
                ms,
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
        async openNearestAccess(access, _log) {
            if ((access?.name ?? 'Bank booth').toLowerCase() !== 'bank booth' ||
                (access?.op ?? 'Use-quickly').toLowerCase() !== 'use-quickly') {
                throw notImpl('Bank.openNearestAccess', 'unsupported bank access');
            }
            const row = snap().nearest_booth;
            if (!row) return false;
            const adjacent = () => {
                const h = snap().here;
                return h && h.level === (row.level ?? 0) && Math.max(Math.abs(h.x-row.x), Math.abs(h.z-row.z)) <= 1;
            };
            if (!adjacent()) {
                queue({op:'walk-near',x:row.x,z:row.z,level:row.level ?? 0,radius:1,allow_teleports:false});
                if (!(await Execution.delayUntil(adjacent, 60000))) return false;
            }
            return Bank.openBooth();
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
