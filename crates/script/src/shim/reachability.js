import { notV1, proxy } from '../../../shim/_kernel.js';

export const Reachability = proxy('Reachability', {
    canReach(_target, _opts = {}) {
        throw notV1('Reachability.canReach');
    },
});
