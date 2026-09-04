// Posted tick only. No attach, no packet listener.
import { snap, proxy } from '../shim/_kernel.js';

export const BotHost = proxy('BotHost', {
    get tickCount() {
        return snap().tick ?? 0;
    },
});
