// Posted tick only. No attach, no packet listener.
import { snap, proxy } from '../shim/_kernel.js';

const tickListeners = new Set();

function addTickListener(cb) {
    tickListeners.add(cb);
    return () => tickListeners.delete(cb);
}

globalThis.__rs2b0t_fire_tick_listeners = () => {
    for (const cb of tickListeners) {
        try {
            cb();
        } catch (err) {
            try {
                const h = globalThis.__rs2b0t_host;
                if (h) {
                    h.log = h.log || [];
                    h.log.push('[rs2b0t] listener error ' + String((err && err.message) || err));
                }
            } catch (_) {}
            try {
                console.error('[rs2b0t] listener error', err);
            } catch (_) {}
        }
    }
};

export const BotHost = proxy('BotHost', {
    get tickCount() {
        return snap().tick ?? 0;
    },
    addTickListener,
});
