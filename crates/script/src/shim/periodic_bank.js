// PeriodicBank Task: ABI matches frozen callers. Rust owns due evaluation,
// Pause/hold clocks, backoff and the walk/open/deposit/return sequence.
// Callbacks stay script-owned. Do not call Banking.bankNearest.
import { Bank } from '../bank/Bank.js';
import { depositMatcher } from '../bank/Banking.js';
import { Execution } from '../execution/Execution.js';
import { Game } from '../game/Game.js';
import { distanceTo, queue, snap } from '../../shim/_kernel.js';

const WALK_MS = 60000;
const BANK_WAIT_MS = 4000;

function timeoutMs(step, fallback) {
    const n = Number(step && step.timeout_ms);
    return Number.isFinite(n) && n > 0 ? n : fallback;
}

function call(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_periodic_bank(payload);
}

function bankApproaches(s) {
    const rows = s.bank_approaches;
    if (!Array.isArray(rows)) {
        return [];
    }
    return rows.map((row) => ({
        loc_id: row.loc_id,
        x: row.x,
        z: row.z,
        level: row.level ?? 0,
        can_operate: !!row.can_operate,
        dest_ok: !!row.dest_ok,
        dest_x: row.dest_x,
        dest_z: row.dest_z,
        dest_level: row.dest_level ?? 0,
    }));
}

function obs() {
    const s = snap();
    const booth = s.nearest_booth || null;
    return {
        here: s.here || null,
        bank_open: Bank.isOpen(),
        bank_loaded: Bank.snapshotReady(),
        bank_generation: Bank.snapshotGeneration(),
        nearest_booth: booth,
        booth_name: booth && booth.name ? String(booth.name) : undefined,
        booth_action: booth && booth.op ? String(booth.op) : undefined,
        has_booth_stands: (s.banks || []).some((stand) => stand && stand.kind === 'booth'),
        bank_approaches: bankApproaches(s),
    };
}

function adjacent(tile, radius) {
    const here = snap().here;
    return here && tile && (here.level ?? 0) === (tile.level ?? 0) && distanceTo(here, tile) <= radius;

}

export class PeriodicBank {
    constructor(opts) {
        this.opts = opts || {};
    }

    validate() {
        const strategy =
            typeof this.opts.strategy === 'function' ? this.opts.strategy() : 'off';
        const lootCount =
            typeof this.opts.countLoot === 'function' ? Number(this.opts.countLoot()) || 0 : 0;
        const itemsThreshold =
            typeof this.opts.itemsThreshold === 'function'
                ? Number(this.opts.itemsThreshold()) || 0
                : 15;
        const minutesThreshold =
            typeof this.opts.minutesThreshold === 'function'
                ? Number(this.opts.minutesThreshold()) || 0
                : 10;
        return call({
            op: 'validate',
            strategy,
            in_combat: Game.inCombat(),
            loot_count: lootCount,
            items_threshold: itemsThreshold,
            minutes_threshold: minutesThreshold,
        }) === true;
    }

    async execute() {
        if (!this.validate()) {
            return;
        }
        this.opts.setStatus?.('periodic bank run');
        const dest = typeof this.opts.destination === 'function' ? this.opts.destination() : null;
        const returnTo = typeof this.opts.returnTo === 'function' ? this.opts.returnTo() : null;
        const started = call({
            op: 'begin',
            destination: dest,
            return_to: returnTo,
            has_after_deposit: typeof this.opts.afterDeposit === 'function',
            npc_access: !!(dest && dest.npcAccess),
            bank_generation: Bank.snapshotGeneration(),
        });
        const token = started.token;
        const aborted = () => call({ op: 'current_token' }) !== token;
        const fail = async () => {
            if (!aborted()) {
                call({ op: 'fail', token });
                this.opts.log?.('periodic bank: no bank reachable — will retry later');
                await Execution.delayTicks(3);
            }
        };
        if (started.kind === 'fail') {
            await fail();
            return;
        }
        for (;;) {
            if (aborted()) {
                return;
            }
            let step = call({ op: 'next', token, ...obs() });
            stepLoop: for (;;) {
                if (aborted()) {
                    return;
                }
                switch (step.kind) {
                case 'aborted':
                    return;
                case 'wait': {
                    const ms = timeoutMs(step, BANK_WAIT_MS);
                    let progressed = false;
                    const settled = await Execution.delayUntil(() => {
                        if (aborted()) {
                            return true;
                        }
                        const next = call({ op: 'next', token, ...obs() });
                        if (next.kind !== 'wait') {
                            step = next;
                            progressed = true;
                            return true;
                        }
                        return false;
                    }, ms);
                    if (aborted()) {
                        return;
                    }
                    if (!settled || (!progressed && step.kind === 'wait')) {
                        await fail();
                        return;
                    }
                    continue stepLoop;
                }
                case 'walk-near': {
                    const tile = { x: step.x, z: step.z, level: step.level };
                    queue({
                        op: 'walk-near',
                        x: step.x,
                        z: step.z,
                        level: step.level,
                        radius: step.radius,
                        allow_teleports: false,
                        allow_wilderness: true,
                        allow_bank_fetch: true,
                    });
                    const ok = await Execution.delayUntil(
                        () => aborted() || adjacent(tile, step.radius),
                        timeoutMs(step, WALK_MS),
                    );
                    if (aborted()) return;
                    if (!ok) {
                        await fail();
                        return;
                    }
                    break stepLoop;
                }
                case 'walk-nearest-bank': {
                    queue({ op: 'walk-nearest-bank' });
                    const ok = await Execution.delayUntil(() => {
                        if (aborted()) return true;
                        const booth = snap().nearest_booth;
                        return booth && adjacent(booth, 1);
                    }, Number(step.timeout_ms) || WALK_MS);
                    if (aborted()) return;
                    if (!ok) {
                        await fail();
                        return;
                    }
                    break stepLoop;
                }
                case 'open-booth': {
                    const baseline = Bank.snapshotGeneration();
                    const req = {
                        op: 'open-booth',
                        x: step.x,
                        z: step.z,
                        level: step.level,
                        id: step.id,
                    };
                    if (step.name !== undefined) req.name = step.name;
                    if (step.action !== undefined) req.action = step.action;
                    queue(req);
                    const ready = await Execution.delayUntil(
                        () =>
                            aborted() ||
                            (Bank.snapshotReady() && Bank.snapshotGeneration() > baseline),
                        timeoutMs(step, BANK_WAIT_MS),
                    );
                    if (aborted()) return;
                    if (!ready) {
                        await fail();
                        return;
                    }
                    break stepLoop;
                }
                case 'deposit': {
                    if (typeof this.opts.deposit !== 'function') {
                        await fail();
                        return;
                    }
                    const commonJunk =
                        typeof this.opts.commonJunk === 'function' ? !!this.opts.commonJunk() : true;
                    await Bank.depositAllMatching(depositMatcher(this.opts.deposit, commonJunk));
                    if (aborted()) return;
                    if (call({ op: 'note_deposit', token }).kind === 'aborted') return;
                    break stepLoop;
                }
                case 'after_deposit': {
                    await this.opts.afterDeposit?.();
                    if (aborted()) return;
                    if (call({ op: 'ack_after_deposit', token }).kind === 'aborted') return;
                    break stepLoop;
                }
                case 'close': {
                    await Bank.close();
                    if (aborted()) return;
                    break stepLoop;
                }
                case 'return': {
                    const tile = { x: step.x, z: step.z, level: step.level };
                    queue({
                        op: 'walk-near',
                        x: step.x,
                        z: step.z,
                        level: step.level,
                        radius: step.radius,
                        allow_teleports: false,
                        allow_wilderness: true,
                        allow_bank_fetch: true,
                    });
                    const ok = await Execution.delayUntil(
                        () => aborted() || adjacent(tile, step.radius),
                        timeoutMs(step, WALK_MS),
                    );
                    if (aborted()) return;
                    if (!ok) {
                        await fail();
                        return;
                    }
                    break stepLoop;
                }
                case 'ok':
                    call({ op: 'ok', token });
                    this.opts.log?.('periodic bank: completed');
                    return;
                case 'fail':
                    await fail();
                    return;
                default:
                    await fail();
                    return;
                }
            }
        }
    }
}
