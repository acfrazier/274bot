import type { NativeApi } from '../host-js/index.d.ts';

/** Read-only published-woods placements. Not an iron action. Not F-PROOF. */
export const apiVersion = 2;

export function tick(api: NativeApi): void {
    const placements = api.gatherPlacements({
        resource: 'oak',
        region: { min_x: 2355, min_z: 3412, max_x: 2356, max_z: 3425, level: 0 },
        limit: 64,
    });
    api.log(JSON.stringify(placements));
}
