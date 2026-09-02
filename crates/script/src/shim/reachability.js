import { snap, proxy, chebyshev } from '../../../shim/_kernel.js';

export const Reachability = proxy('Reachability', {
    canReach(target, opts = {}) {
        const here = snap().here;
        if (!here || !target) return false;
        const maxSteps = opts.maxSteps ?? 400;
        return chebyshev(here, target) <= maxSteps;
    },
});
