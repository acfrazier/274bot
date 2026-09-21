import { Reachability } from '../../event/webwalk/geometry/Reachability.js';

type NativeApi = import('../host-js/index.d.ts').NativeApi;

/**
 * Headed File witness: observed one-step cardinal pairs, then real
 * NativeApi.lineOfSight and frozen Reachability.lineOfSight. Selection
 * uses raw V-direction bits only; it does not copy the DDA.
 */
export const apiVersion = 2;

const STOP_OK = 'line of sight qualification complete';
const WALK_SCENERY = 0x100;
const V_N = 0x400;
const V_E = 0x1000;
const V_S = 0x4000;
const V_W = 0x10000;
const RADIUS = 8;
const DIRS: Array<[number, number, number]> = [
    [1, 0, V_W],
    [-1, 0, V_E],
    [0, 1, V_S],
    [0, -1, V_N],
];

type Tile = { x: number; z: number; level: number };
type Pair = {
    from: Tile;
    to: Tile;
    src: number;
    dst: number;
    mask: number;
};

function flagAt(
    c: NativeApi['snapshot']['collision'],
    x: number,
    z: number,
): number | undefined {
    const lx = x - c.base_x;
    const lz = z - c.base_z;
    if (lx < 0 || lz < 0 || lx >= c.width || lz >= c.height) return undefined;
    return c.flags.at(lx * c.height + lz);
}

function chebyshev(a: Tile, b: Tile): number {
    return Math.max(Math.abs(a.x - b.x), Math.abs(a.z - b.z));
}

function selectPairs(here: Tile, c: NativeApi['snapshot']['collision']): { open: Pair; blocked: Pair } | string {
    let open: Pair | undefined;
    let blocked: Pair | undefined;
    for (let r = 0; r <= RADIUS; r++) {
        for (let dx = -r; dx <= r; dx++) {
            for (let dz = -r; dz <= r; dz++) {
                if (Math.max(Math.abs(dx), Math.abs(dz)) !== r) continue;
                const from = { x: here.x + dx, z: here.z + dz, level: here.level };
                if (chebyshev(here, from) > RADIUS) continue;
                const src = flagAt(c, from.x, from.z);
                if (src === undefined) continue;
                for (const [sx, sz, mask] of DIRS) {
                    const to = { x: from.x + sx, z: from.z + sz, level: here.level };
                    if (chebyshev(here, to) > RADIUS) continue;
                    const dst = flagAt(c, to.x, to.z);
                    if (dst === undefined) continue;
                    // Source WALK_SCENERY clear on both pairs: scenery on the
                    // source makes LOS return false before wall tracing.
                    if (
                        !blocked &&
                        (src & WALK_SCENERY) === 0 &&
                        (dst & mask) !== 0
                    ) {
                        blocked = { from, to, src, dst, mask };
                    } else if (!open && (src & WALK_SCENERY) === 0 && (dst & mask) === 0) {
                        open = { from, to, src, dst, mask };
                    }
                    if (open && blocked) return { open, blocked };
                }
            }
        }
    }
    return 'los fixture failure: no cardinal open+blocked V-wall pair within 8';
}

export function tick(api: NativeApi): void {
    const here = api.snapshot.here;
    const c = api.snapshot.collision;
    if (!api.snapshot.ingame || !here) return;
    if (!c.available) return;
    if (here.level !== c.level) return;
    const selfFlag = flagAt(c, here.x, here.z);
    if (selfFlag === undefined) return;

    const selected = selectPairs(here, c);
    if (typeof selected === 'string') {
        throw new Error(selected);
    }
    const { open, blocked } = selected;

    const openAt = c.flags.at((open.from.x - c.base_x) * c.height + (open.from.z - c.base_z));
    const blockedAt = c.flags.at((blocked.to.x - c.base_x) * c.height + (blocked.to.z - c.base_z));
    if (openAt !== open.src || blockedAt !== blocked.dst) {
        throw new Error('flags.at does not match selected pair cells');
    }

    const openV2 = api.lineOfSight({ from: open.from, to: open.to, size: 1 });
    const blockedV2 = api.lineOfSight({ from: blocked.from, to: blocked.to, size: 1 });
    if (!openV2.ok) throw new Error(openV2.error);
    if (!blockedV2.ok) throw new Error(blockedV2.error);
    if (openV2.value !== true) throw new Error('open pair v2 must be true');
    if (blockedV2.value !== false) throw new Error('blocked pair v2 must be false');

    const openV1 = Reachability.lineOfSight(open.from, open.to, 1);
    const blockedV1 = Reachability.lineOfSight(blocked.from, blocked.to, 1);
    if (openV1 !== true || blockedV1 !== false) {
        throw new Error('v1 must agree with the same observed pairs');
    }

    const receipt = {
        identity: {
            base_x: c.base_x,
            base_z: c.base_z,
            level: c.level,
            width: c.width,
            height: c.height,
        },
        here: { x: here.x, z: here.z, level: here.level, flag: selfFlag },
        open: {
            from: open.from,
            to: open.to,
            src: open.src,
            dst: open.dst,
            mask: open.mask,
            v2: openV2.value,
            v1: openV1,
        },
        blocked: {
            from: blocked.from,
            to: blocked.to,
            src: blocked.src,
            dst: blocked.dst,
            mask: blocked.mask,
            v2: blockedV2.value,
            v1: blockedV1,
        },
    };
    const line = `los-receipt:${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint
        .begin()
        .title('line of sight v2')
        .row(line)
        .row(
            'open',
            `${open.from.x},${open.from.z}->${open.to.x},${open.to.z}`,
            'v2',
            openV2.value,
            'v1',
            openV1,
        )
        .row(
            'blocked',
            `${blocked.from.x},${blocked.from.z}->${blocked.to.x},${blocked.to.z}`,
            'mask',
            blocked.mask,
            'v2',
            blockedV2.value,
            'v1',
            blockedV1,
        )
        .row('result', STOP_OK)
        .end();
    (globalThis as { __losExample?: unknown }).__losExample = {
        ok: true,
        open: openV2.value,
        blocked: blockedV2.value,
        selfFlag,
        receipt,
    };
    api.stop(STOP_OK);
}
