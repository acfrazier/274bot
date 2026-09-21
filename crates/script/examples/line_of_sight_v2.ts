type NativeApi = import('../host-js/index.d.ts').NativeApi;

/** Offline LOS example: raw flags.at plus a true and a false query. */
export const apiVersion = 2;

export function tick(api: NativeApi): void {
    const here = api.snapshot.here;
    const c = api.snapshot.collision;
    if (!here) throw new Error('missing here');
    if (!c.available) throw new Error('missing-observation');
    const lx = here.x - c.base_x;
    const lz = here.z - c.base_z;
    if (lx < 0 || lz < 0 || lx >= c.width || lz >= c.height) {
        throw new Error('here out of collision');
    }
    const selfFlag = c.flags.at(lx * c.height + lz);
    if (selfFlag === undefined) throw new Error('self flag missing');
    const open = api.lineOfSight({
        from: here,
        to: { x: here.x + 2, z: here.z, level: here.level },
        size: 1,
    });
    if (!open.ok) throw open.error;
    const blocked = api.lineOfSight({
        from: here,
        to: { x: here.x + 7, z: here.z, level: here.level },
        size: 1,
    });
    if (!blocked.ok) throw blocked.error;
    const planes = api.lineOfSight({
        from: here,
        to: { x: here.x, z: here.z, level: here.level + 1 },
        size: 1,
    });
    if (!planes.ok) throw planes.error;
    if (planes.value !== false) throw new Error('different plane must be false');
    (globalThis as { __losExample?: unknown }).__losExample = {
        ok: open.value === true && blocked.value === false && selfFlag === 0,
        open: open.value,
        blocked: blocked.value,
        selfFlag,
    };
}
