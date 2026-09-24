// SolveClue Task: marshal only. `validate` is the embed's enabled gate plus one
// begin; `execute` awaits the isolate clue family. Rust owns the token, the
// identify, the phases, the clocks, the scene reads and the verbs. Callbacks
// (`enabled` / `log` / `setStatus`) go through the one callback path. This file
// never echoes snapshot pages and never enqueues a loc for an unknown kind.
import { notImpl, runMachine } from '../../../shim/_kernel.js';
import { Traversal } from '../../walking/Traversal.js';

const throwUse = (name) => {
    throw notImpl(name);
};

function clueCall(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_clue
        : undefined;
    if (typeof fn !== 'function') return null;
    return fn(payload);
}

const TERMINAL = new Set(['done', 'dead', 'abandon', 'guardian-lost', 'aborted']);

export class SolveClue {
    constructor(hostArg) {
        // Coerce and store only: the seven embeds construct this in onStart,
        // where a begin with no held membership would be refused anyway. The
        // constructor never throws and never begins.
        this.host = hostArg && typeof hostArg === 'object' ? hostArg : {};
        this.token = null;
        this.status = 'idle';
    }

    clueStatus() {
        // The machine's own `callback.setStatus` line, and only that: 'idle'
        // until one is dispatched.
        return this.status;
    }

    noteDeath() {
        // Callbacks stay script owned; the death latch is not this file's.
    }

    ownsEquipment() {
        // The machine's own stripped-gear read, and only that: a read of rust
        // state and never a policy of this file. True means do-not-grind-equip
        // (the frozen GearEquip consumers), and it stays true across a dead
        // token while the names are unclaimed.
        const answer = clueCall({ op: 'ownsEquipment' });
        return !!(answer && answer.owns === true);
    }

    retry() {
        // The landed `SolveClue.retry()`: the machine's own latch clear, never
        // `on_reset`. The live token is not aborted and the stripped list is
        // not touched — only the abandon latch clears.
        const answer = clueCall({ op: 'retry' });
        return !!(answer && answer.kind === 'retry');
    }

    // The embed's own gate, coerced the way the callers coerce it: absent is
    // enabled, and a false answer only means "do not run this session".
    enabled() {
        const fn = this.host.enabled;
        const value = typeof fn === 'function' ? fn() : undefined;
        return value === undefined || value === null ? true : !!value;
    }

    hooks() {
        return {
            enabled: () => this.enabled(),
            log: (message) => {
                this.host.log?.(String(message));
            },
            setStatus: (message) => {
                const text = String(message);
                this.status = text;
                this.host.setStatus?.(text);
            },
        };
    }

    validate() {
        // Disabled first: it must not begin, and it must not abort a live
        // token either — the same session resumes when the flag comes back.
        if (!this.enabled()) return false;
        // A live token is the machine's to keep: a second begin aborts it.
        if (this.token !== null) return true;
        const begin = clueCall({ op: 'begin' });
        if (!begin || begin.kind !== 'token') return false;
        this.token = begin.token;
        return true;
    }

    async execute() {
        if (this.token === null) {
            // Only reached when a caller runs execute without validate: the
            // one begin is the same begin validate makes.
            const begin = clueCall({ op: 'begin' });
            if (!begin || begin.kind !== 'token') return;
            this.token = begin.token;
        }
        const out = await runMachine('clue', { token: this.token }, this.hooks());
        if (out.kind === 'refused' || out.kind === 'aborted') {
            this.token = null;
            return;
        }
        if (out.kind === 'done') {
            const value = out.value;
            const kind = value && typeof value === 'object' ? value.kind : '';
            // Yield keeps the token live. Terminals kill it. An unknown kind
            // is not a loc and not a verb: leave the token as the machine left it.
            if (TERMINAL.has(kind)) this.token = null;
        }
    }
}

export function heldClueLikeId() {
    throwUse('SolveClue.heldClueLikeId');
}

// Frozen `walkToBank` (SolveClue.ts:96-105): one resilient trail-leg walk to
// the bank stand (radius 3, 300 s, the trail's default teleport policy).
// The Isafdar and Kharazi crossings are not mapped; the host route decides.
export function walkToBank(tile, log) {
    return Traversal.walkResilient(tile, {
        radius: 3,
        attempts: 6,
        timeoutMs: 300_000,
        log,
        useTeleportCatalog: true,
        policy: { useTeleports: true, distanceBeforeTeleport: 40 },
    });
}
