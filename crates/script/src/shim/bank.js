import { runMachine } from '../../shim/_kernel.js';
const notImpl = (name, reason) =>
    new Error(reason ? 'not impl: ' + name + ': ' + reason : 'not impl: ' + name);

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
            return globalThis.__rs2b0t_bank_snapshot('isOpen');
        },
        loaded() {
            return globalThis.__rs2b0t_bank_snapshot('loaded');
        },
        ready() {
            return globalThis.__rs2b0t_bank_snapshot('ready');
        },
        items() {
            return globalThis.__rs2b0t_bank_snapshot('items');
        },
        count(name) {
            return globalThis.__rs2b0t_bank_snapshot('count', String(name));
        },
        deposit(name, op = 'Deposit-1') {
            return bankOp({ kind: 'deposit', name: String(name), op: String(op) });
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
        // Rust owns the note intent for the open bank session; a closed
        // bank is a no-op, as frozen. Frozen returns Promise<void>.
        async setNoteMode(on) {
            await runMachine('bank_note_mode', { on: !!on });
        },
        // Rust sends the Close and waits for the posted acknowledgement;
        // an omitted bound is Rust's default.
        async close(timeoutMs) {
            const out = await runMachine('bank_close', {
                timeout_ms: timeoutMs === undefined ? null : Number(timeoutMs),
            });
            return out.kind === 'done' && out.value === true;
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
            const out = await runMachine(
                'bank_wait',
                { kind: 'ready', timeout_ms: ms },
                { log: typeof log === 'function' ? log : undefined },
            );
            return out.kind === 'done' && out.value === true;
        },
        snapshotReady() {
            return globalThis.__rs2b0t_bank_snapshot('snapshotReady');
        },
        snapshotGeneration() {
            return globalThis.__rs2b0t_bank_snapshot('snapshotGeneration');
        },
        async waitSnapshotAfter(generation, timeoutMs) {
            const baseline = Number(generation);
            if (!Number.isFinite(baseline) || baseline < 0) {
                return false;
            }
            const ms = typeof timeoutMs === 'number' && timeoutMs > 0 ? timeoutMs : 5000;
            const out = await runMachine('bank_wait', {
                kind: 'snapshot-after',
                timeout_ms: ms,
                baseline_generation: baseline,
            });
            return out.kind === 'done' && out.value === true;
        },
        countById(id) {
            return globalThis.__rs2b0t_bank_snapshot('countById', Number(id));
        },
        // Rust picks the row's Withdraw-All or the fill ladder and settles
        // on the posted pack.
        async withdrawLoad(name) {
            const out = await runMachine('bank_withdraw_load', { name: String(name) });
            return out.kind === 'done' && out.value === true;
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
