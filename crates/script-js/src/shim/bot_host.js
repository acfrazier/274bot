// Posted tick only. No attach, no packet listener.
import { snap, proxy, notImpl } from '../shim/_kernel.js';

const TICK_EVENT = '__rs2b0t_internal_tick__';

function addTickListener(cb) {
    const id = globalThis.__rs2b0t_native_event_on(TICK_EVENT, cb);
    return () => globalThis.__rs2b0t_native_event_off(id);
}
function addFrameListener(cb) {
    const id = globalThis.__rs2b0t_native_event_on('frame', cb);
    return () => globalThis.__rs2b0t_native_event_off(id);
}

export const BotHost = proxy('BotHost', {
    get tickCount() {
        return snap().tick ?? 0;
    },
    addTickListener,
    addFrameListener,
});
export class BotHostImpl {
    constructor() {
        throw notImpl('BotHostImpl');
    }
}

