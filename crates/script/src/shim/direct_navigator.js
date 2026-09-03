import { queue, proxy, notV1 } from '../../shim/_kernel.js';

// Same-scene walk click. `walk-to` is the scene packet; Traveller is `walk`.
export const DirectNavigator = proxy('DirectNavigator', {
    walk(dest) {
        if (!dest || typeof dest.x !== 'number' || typeof dest.z !== 'number') return false;
        queue({
            op: 'walk-to',
            x: dest.x,
            z: dest.z,
            level: dest.level ?? 0,
        });
        return true;
    },
    walkTo() {
        throw notV1('DirectNavigator.walkTo');
    },
});
