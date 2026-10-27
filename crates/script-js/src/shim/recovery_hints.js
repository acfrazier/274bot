import Tile from '../geometry/Tile.js';

// Frozen RecoveryHints (`RecoveryHints.ts:3-18`). The slot owns the state so
// it outlives a watchdog isolate restart; each member is one native call.
function hints(op, value) {
    return globalThis.__rs2b0t_recovery_hints(op, value);
}

function tile(t) {
    return t ? Tile.from(t) : null;
}

export const RecoveryHints = {
    get pendingRecovery() {
        return hints('pending');
    },
    set pendingRecovery(value) {
        hints('set-pending', value);
    },
    get anchor() {
        return tile(hints('anchor'));
    },
    set anchor(value) {
        hints('set-anchor', value ?? null);
    },
    takeAnchor() {
        return tile(hints('take'));
    },
    clear() {
        hints('clear');
    },
};
