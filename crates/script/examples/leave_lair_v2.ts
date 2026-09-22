type NativeApi = import('../host-js/index.d.ts').NativeApi;

/**
 * Headed File witness: field observation of the gateless walk-out first
 * effect. Not a lair exit. The player starts inside a curated box (here ± 2,
 * same level). walkOut is outside that box, same level, Chebyshev 8–16.
 * One leaveNext must be walk-near radius 3 toward that walkOut. Do not ack
 * the walk. Do not call the ack-and-wait helper. Do not queue. Do not invent allow-flags.
 * Do not claim the player left the box. Already outside is a different
 * discriminator and is not this cell.
 * Gate is ingame and here only. The v2 snapshot does not publish a scene field.
 */
export const apiVersion = 2;

const STOP_OK = 'leave lair qualification complete';
const RECEIPT_PREFIX = 'leave-lair-receipt:';
const MIN_CHEB = 8;
const MAX_CHEB = 16;
const BOX_PAD = 2;
const SITE_KEY = 'leave-lair';
const EDGEVILLE = { x: 3094, z: 3493 };
const VARROCK_LAND = { x: 3213, z: 3424 };
const KBD_TILE = { x: 3017, z: 3849 };

type Tile = { x: number; z: number; level: number };
type Box = { minX: number; maxX: number; minZ: number; maxZ: number; level: number };

let token: number | null = null;
let walkOut: Tile | null = null;
let box: Box | null = null;

function chebyshev(a: Tile, b: Tile): number {
    return Math.max(Math.abs(a.x - b.x), Math.abs(a.z - b.z));
}

function sameTile(a: Tile, b: Tile): boolean {
    return a.x === b.x && a.z === b.z && a.level === b.level;
}

function contains(area: Box, tile: Tile): boolean {
    return (
        tile.level === area.level &&
        tile.x >= area.minX &&
        tile.x <= area.maxX &&
        tile.z >= area.minZ &&
        tile.z <= area.maxZ
    );
}

function forbiddenTile(tile: Tile): boolean {
    return (
        (tile.x === EDGEVILLE.x && tile.z === EDGEVILLE.z) ||
        (tile.x === VARROCK_LAND.x && tile.z === VARROCK_LAND.z) ||
        (tile.x === KBD_TILE.x && tile.z === KBD_TILE.z)
    );
}

/** Curated fixture box. here ± 2 on the same level contains the start tile. */
function curatedBox(here: Tile): Box {
    return {
        minX: here.x - BOX_PAD,
        maxX: here.x + BOX_PAD,
        minZ: here.z - BOX_PAD,
        maxZ: here.z + BOX_PAD,
        level: here.level,
    };
}

function pickWalkOut(here: Tile): { walkOut: Tile; box: Box } | null {
    if (forbiddenTile(here)) return null;
    const area = curatedBox(here);
    if (!contains(area, here)) return null;
    for (let r = MIN_CHEB; r <= MAX_CHEB; r++) {
        for (let dx = -r; dx <= r; dx++) {
            for (let dz = -r; dz <= r; dz++) {
                if (Math.max(Math.abs(dx), Math.abs(dz)) !== r) continue;
                const cand = { x: here.x + dx, z: here.z + dz, level: here.level };
                if (sameTile(cand, here) || forbiddenTile(cand)) continue;
                if (cand.level !== here.level) continue;
                const gap = chebyshev(here, cand);
                if (gap < MIN_CHEB || gap > MAX_CHEB) continue;
                if (contains(area, cand) || !contains(area, here)) continue;
                return { walkOut: cand, box: area };
            }
        }
    }
    return null;
}

function forbiddenKind(kind: string | undefined): boolean {
    if (typeof kind !== 'string') return false;
    const k = kind.toLowerCase();
    return (
        k === 'walk' ||
        k === 'teleport' ||
        k === 'loc' ||
        k === 'yield' ||
        k === 'aborted' ||
        k === 'kbd'
    );
}

export function tick(api: NativeApi): void {
    const snap = api.snapshot;
    if (!snap.ingame || !snap.here) {
        return;
    }
    const here = snap.here;
    if (forbiddenTile(here)) {
        return;
    }

    if (!walkOut || !box || !contains(box, here) || contains(box, walkOut)) {
        const picked = pickWalkOut(here);
        if (!picked) {
            return;
        }
        walkOut = picked.walkOut;
        box = picked.box;
    }
    const gap = chebyshev(here, walkOut);
    if (
        here.level !== walkOut.level ||
        gap < MIN_CHEB ||
        gap > MAX_CHEB ||
        forbiddenTile(walkOut) ||
        !contains(box, here) ||
        contains(box, walkOut)
    ) {
        walkOut = null;
        box = null;
        return;
    }

    const projection = {
        leaveByWalk: true,
        key: SITE_KEY,
        boxes: [box],
        walkOut: { x: walkOut.x, z: walkOut.z, level: walkOut.level },
    };
    if (projection.leaveByWalk !== true) {
        throw new Error('leave lair witness must stay gateless');
    }

    if (token == null) {
        const began = api.leaveBegin();
        if (!began.ok) {
            throw new Error(began.error);
        }
        token = began.value.token;
    }

    for (let i = 0; i < 4; i++) {
        const step = api.leaveNext({ token, ...projection });
        if (!step.ok) {
            throw new Error(step.error);
        }
        const kind = 'kind' in step ? step.kind : undefined;
        if (kind === 'yield') {
            throw new Error('leave lair witness must not claim the player left the box');
        }
        if (forbiddenKind(kind)) {
            throw new Error(
                'leave lair witness must not emit walk / teleport / loc / yield / KBD',
            );
        }
        if (kind === 'status' || kind === 'log' || kind === 'wait' || kind === 'sustain') {
            continue;
        }
        if (kind !== 'walk-near') {
            throw new Error(`leaveNext expected walk-near radius 3, got ${String(kind)}`);
        }
        const walk = step as {
            x?: number;
            z?: number;
            level?: number;
            radius?: number;
        };
        if (walk.x !== walkOut.x || walk.z !== walkOut.z || walk.level !== walkOut.level) {
            throw new Error('leaveNext walk-near must target walkOut');
        }
        if (walk.radius !== 3) {
            throw new Error('leaveNext walk-near must be radius 3');
        }

        const receipt = {
            here: { x: here.x, z: here.z, level: here.level },
            walkOut: { x: walkOut.x, z: walkOut.z, level: walkOut.level },
            kind,
            radius: walk.radius,
            discriminator: 'gateless',
        };
        const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
        api.log(line);
        api.paint
            .begin()
            .title('leave lair v2')
            .row(line)
            .row(
                'here',
                `${here.x},${here.z},${here.level}`,
                'walkOut',
                `${walkOut.x},${walkOut.z},${walkOut.level}`,
            )
            .row('kind', String(kind), 'discriminator', 'gateless')
            .row('result', STOP_OK)
            .end();
        api.stop(STOP_OK);
        return;
    }
    throw new Error('leaveNext did not emit walk-near radius 3');
}
