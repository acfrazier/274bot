// Our Execution module: delay / delayTicks / delayUntil park the loop's
// await. The isolate does not call `loop()` again while a wait is active —
// the tick loop settles the wait (`__rs2b0t_pump`) on each posted
// PLAYER_INFO tick instead. Guardian hold / Pause freezes the cond too:
// the tick loop skips the pump while held/paused, so even time waits stay
// parked.
const host = () => globalThis.__rs2b0t_host || {};

// One parked wait (the bot loop is sequential: one await at a time).
// Settled by `__rs2b0t_pump` on each posted tick; wall-clock waits use
// isolate time (performance.now()), like the rs2b0t Scheduler.
const park = {
    active: null,

    enqueue(spec) {
        return new Promise((resolve, reject) => {
            park.active = { ...spec, resolve, reject };
            host().parked = true;
        });
    },

    // Settle a due wait. Mirrors rs2b0t Scheduler.trySettle order: cond,
    // then timeout. The wait stays parked until the host pumps again.
    settle(tick, now) {
        const wait = park.active;
        if (!wait) {
            return;
        }
        const done = (value) => {
            park.active = null;
            host().parked = false;
            wait.resolve(value);
        };
        if (wait.kind === 'tick') {
            if (tick >= wait.dueTick) done(true);
            return;
        }
        if (wait.kind === 'time') {
            if (now >= wait.dueAt) done(true);
            return;
        }
        try {
            if (wait.cond()) {
                done(true);
                return;
            }
        } catch (err) {
            park.active = null;
            host().parked = false;
            wait.reject(err instanceof Error ? err : new Error(String(err)));
            return;
        }
        if (wait.timeoutAt !== null && now >= wait.timeoutAt) done(false);
    },
};

export const Execution = {
    async delay(ms) {
        await park.enqueue({ kind: 'time', dueAt: performance.now() + ms });
    },

    async delayTicks(n) {
        await park.enqueue({
            kind: 'tick',
            dueTick: (host().tick || 0) + Math.max(0, Math.floor(n)),
        });
    },

    delayUntil(cond, timeoutMs = 6000) {
        if (typeof cond !== 'function') {
            return Promise.reject(new Error('not impl: Execution.delayUntil: requires a function'));
        }
        return park.enqueue({
            kind: 'cond',
            cond,
            timeoutAt: timeoutMs > 0 ? performance.now() + timeoutMs : null,
        });
    },

    delayUntilTicks(_cond, _maxTicks) {
        return Promise.reject(new Error('not impl: Execution.delayUntilTicks'));
    },
};

// The isolate tick loop calls this (through the rustyscript event loop,
// so the resolved wait's continuation runs) on every posted tick while a
// wait is parked: settle due waits, and the loop's continuation either
// completes the tick or parks again.
globalThis.__rs2b0t_pump = async (n) => {
    const state = host();
    state.tick = n;
    park.settle(n, performance.now());
};
