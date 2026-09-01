// Our Game module. `ingame`/`tile` read the host-posted snapshot blob
// (`__rs2b0t_host.snapshot`; missing → false/null); `tick` is the
// host-posted PLAYER_INFO counter set by the tick wrapper/pump each posted
// tick (so Execution delayUntil conds can wait on it); `teleport` and
// every other unimplemented member throw `not v1` — never a fake value.
const host = () => globalThis.__rs2b0t_host || {};
const notV1 = (name) => new Error('not v1: ' + name);
const snap = () => host().snapshot || {};

export const Game = new Proxy(
    {
        ingame() {
            return snap().ingame === true;
        },
        tile() {
            return snap().here || null;
        },
        tick() {
            return host().tick || 0;
        },
        teleport() {
            throw notV1('Game.teleport');
        },
    },
    {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (prop in target) return target[prop];
            throw notV1('Game.' + String(prop));
        },
    },
);
