// Our Execution module: delay / delayTicks / delayUntil park the caller's
// await until the isolate tick loop settles it (`__rs2b0t_pump`) on a
// posted PLAYER_INFO tick. Any number of waits may be parked at once —
// `loop()`, an `on(event)` callback and an un-awaited helper each hold
// their own — and every wait is settled or rejected, never dropped.
// Guardian hold / Pause freeze them all: the tick loop skips the pump
// while held/paused, so even time waits stay parked.
const host = () => globalThis.__rs2b0t_host || {};

// Mirrors rs2b0t Scheduler.trySettle per wait: cond, then timeout.
// `undefined` keeps the wait parked; an Error rejects it.
function trySettle(wait, tick, now) {
    if (wait.kind === 'tick') return tick >= wait.dueTick ? true : undefined;
    if (wait.kind === 'time') return now >= wait.dueAt ? true : undefined;
    try {
        if (wait.cond()) return true;
    } catch (err) {
        return err instanceof Error ? err : new Error(String(err));
    }
    if (wait.kind === 'cond-ticks') return tick >= wait.dueTick ? false : undefined;
    return wait.timeoutAt !== null && now >= wait.timeoutAt ? false : undefined;
}

// Parked waits in enqueue order; wall-clock waits use isolate time
// (performance.now()), like the rs2b0t Scheduler.
const park = {
    waits: [],

    enqueue(spec) {
        return new Promise((resolve, reject) => {
            park.waits.push({ ...spec, resolve, reject });
            const h = host();
            h.waitEnqueues = (h.waitEnqueues || 0) + 1;
        });
    },

    // A wait enqueued by a settled wait's continuation lands on the next
    // pump: continuations run after this returns, never inside the pass.
    settle(tick, now) {
        if (park.waits.length === 0) return;
        const pending = park.waits;
        park.waits = [];
        const h = host();
        for (const wait of pending) {
            const outcome = trySettle(wait, tick, now);
            if (outcome === undefined) {
                park.waits.push(wait);
                continue;
            }
            h.waitSettles = (h.waitSettles || 0) + 1;
            if (outcome instanceof Error) wait.reject(outcome);
            else wait.resolve(outcome);
        }
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

    delayUntilTicks(cond, maxTicks) {
        if (typeof cond !== 'function') {
            return Promise.reject(
                new Error('not impl: Execution.delayUntilTicks: requires a function'),
            );
        }
        return park.enqueue({
            kind: 'cond-ticks',
            cond,
            dueTick: (host().tick || 0) + Math.max(0, Math.floor(maxTicks)),
            timeoutAt: null,
        });
    },

    // Explicit gameplay+scheduler progress. No timestamp: the host stamps
    // Instant when the generation-matched interact is drained.
    noteProgress() {
        const h = globalThis.__rs2b0t_host;
        if (!h) return;
        h.interact = h.interact || [];
        h.interact.push({ op: 'note-progress' });
    },
};

// The isolate tick loop calls this once per eligible posted tick, at the
// point of its phase order where waits settle (after it recorded the
// tick and fired tick listeners). It only settles due waits; their
// continuations run as the call returns.
globalThis.__rs2b0t_pump = (n) => {
    park.settle(n, performance.now());
};
