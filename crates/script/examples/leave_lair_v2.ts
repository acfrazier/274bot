type NativeApi = import('../host-js/index.d.ts').NativeApi;
type HuntHooks = import('../host-js/index.d.ts').HuntHooks;
type HuntSite = import('../host-js/index.d.ts').HuntSite;

/**
 * Headed File witness: one gateless walk-out. The lair is a curated box
 * (here ± 2); walkOut is Chebyshev 8–16 outside it, never Edgeville /
 * Varrock / KBD. One awaited leaveRun: the host walks near walkOut at
 * radius 3 and the run settles `true` as soon as here is out of the box.
 * Anything but `done` with `true`, or a tile still in the box after the
 * run, throws. The receipt names the Start tile, the tile after the run,
 * walkOut, the site key and the outcome.
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

let walkOut: Tile | null = null;
let box: Box | null = null;
let running = false;

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

export async function tick(api: NativeApi): Promise<void> {
    if (running) {
        return;
    }
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

    const site: HuntSite = {
        key: SITE_KEY,
        boxes: [box],
        walkOut: { x: walkOut.x, z: walkOut.z, level: walkOut.level },
    };
    const hooks: HuntHooks = {
        leaveByWalk: () => true,
    };

    running = true;
    const from = { x: here.x, z: here.z, level: here.level };
    const outcome = await api.leaveRun(site, hooks);
    const after = api.snapshot.here;
    if (outcome.kind !== 'done' || outcome.value !== true || !after || contains(box, after)) {
        throw new Error(`leaveRun did not walk out: ${JSON.stringify(outcome)} at ${JSON.stringify(after)}`);
    }

    const receipt = {
        outcome,
        from,
        here: { x: after.x, z: after.z, level: after.level },
        dest: { x: walkOut.x, z: walkOut.z, level: walkOut.level },
        key: SITE_KEY,
    };
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint
        .begin()
        .title('leave lair v2')
        .row(line)
        .row(
            'from',
            `${from.x},${from.z},${from.level}`,
            'walkOut',
            `${walkOut.x},${walkOut.z},${walkOut.level}`,
        )
        .row('outcome', `${outcome.kind} ${outcome.value}`)
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
