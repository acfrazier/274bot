type NativeApi = import('../host-js/index.d.ts').NativeApi;
type HuntHooks = import('../host-js/index.d.ts').HuntHooks;
type HuntSite = import('../host-js/index.d.ts').HuntSite;

/**
 * Headed File witness: one gateless lair entry. The lair is a 3x3 box
 * around one approach tile Chebyshev 8–16 from here (greater than the skip
 * of 1, not Hold's 2-tile walk-back, not Walk's > 12 gate); here is
 * outside it. begin + validate + one awaited enterRun: the host walks
 * radius 0 to the approach tile with teleports, wilderness and bank fetch
 * off, and the run settles `true` once here is inside the box. Anything
 * but `done` with `true`, or a tile outside the box after the run, throws.
 * The receipt names the Start tile, the tile after the run, the approach
 * tile, the box, the site key and the outcome.
 */
export const apiVersion = 2;

const STOP_OK = 'enter lair qualification complete';
const RECEIPT_PREFIX = 'enter-lair-receipt:';
const MIN_CHEB = 8;
const MAX_CHEB = 16;
const SITE_KEY = 'enter-lair';
const FORBIDDEN_TILE = { x: 3017, z: 3849 };

type Tile = { x: number; z: number; level: number };
type Box = { minX: number; maxX: number; minZ: number; maxZ: number; level: number };

let token: number | null = null;
let approach: Tile | null = null;
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
    return tile.x === FORBIDDEN_TILE.x && tile.z === FORBIDDEN_TILE.z;
}

function walkable(api: NativeApi, tile: Tile): boolean {
    const r = api.walkable({ tile });
    if (!r.ok) return true;
    return r.value === true;
}

/** The lair: the approach tile ± 1. Chebyshev 8–16 keeps `here` outside. */
function approachBox(tile: Tile): Box {
    return {
        minX: tile.x - 1,
        maxX: tile.x + 1,
        minZ: tile.z - 1,
        maxZ: tile.z + 1,
        level: tile.level,
    };
}

function pickApproach(
    here: Tile,
    api: NativeApi,
): { approach: Tile; box: Box } | null {
    for (let r = MIN_CHEB; r <= MAX_CHEB; r++) {
        for (let dx = -r; dx <= r; dx++) {
            for (let dz = -r; dz <= r; dz++) {
                if (Math.max(Math.abs(dx), Math.abs(dz)) !== r) continue;
                const cand = { x: here.x + dx, z: here.z + dz, level: here.level };
                if (sameTile(cand, here)) continue;
                if (forbiddenTile(cand)) continue;
                if (chebyshev(here, cand) < MIN_CHEB || chebyshev(here, cand) > MAX_CHEB) {
                    continue;
                }
                if (!walkable(api, cand)) continue;
                const area = approachBox(cand);
                if (contains(area, here) || !contains(area, cand)) continue;
                return { approach: cand, box: area };
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

    if (approach && sameTile(approach, here)) {
        approach = null;
        box = null;
    }
    if (!approach || !box) {
        const picked = pickApproach(here, api);
        if (!picked) {
            return;
        }
        approach = picked.approach;
        box = picked.box;
    }
    if (contains(box, here)) {
        approach = null;
        box = null;
        return;
    }
    const gap = chebyshev(here, approach);
    if (here.level !== approach.level || gap < MIN_CHEB || gap > MAX_CHEB) {
        approach = null;
        box = null;
        return;
    }
    if (!contains(box, approach) || forbiddenTile(approach) || forbiddenTile(here)) {
        approach = null;
        box = null;
        return;
    }

    const site: HuntSite = {
        key: SITE_KEY,
        boxes: [box],
        approach: [{ x: approach.x, z: approach.z, level: approach.level }],
        talkGate: null,
        feeGate: null,
        gate: null,
        keyItem: null,
    };
    if (site.key === 'kbd-lair') {
        throw new Error('enter lair witness must not use kbd-lair');
    }
    const hooks: HuntHooks = {
        parked: () => false,
        shieldReady: () => true,
        hpFraction: () => 1,
        panicHp: () => 0.2,
    };

    if (token == null) {
        const began = api.enterBegin(site);
        if (!began.ok) {
            throw new Error(began.error);
        }
        token = began.value.token;
    }
    const validated = api.enterValidate({ token }, hooks);
    if (!validated.ok) {
        throw new Error(validated.error);
    }
    if (!validated.value) {
        return;
    }

    running = true;
    const from = { x: here.x, z: here.z, level: here.level };
    const outcome = await api.enterRun({ token }, hooks);
    const after = api.snapshot.here;
    if (outcome.kind !== 'done' || outcome.value !== true || !after || !contains(box, after)) {
        throw new Error(`enterRun did not enter the lair: ${JSON.stringify(outcome)} at ${JSON.stringify(after)}`);
    }

    const receipt = {
        outcome,
        from,
        here: { x: after.x, z: after.z, level: after.level },
        dest: { x: approach.x, z: approach.z, level: approach.level },
        box,
        key: SITE_KEY,
    };
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint
        .begin()
        .title('enter lair v2')
        .row(line)
        .row(
            'from',
            `${from.x},${from.z},${from.level}`,
            'approach',
            `${approach.x},${approach.z},${approach.level}`,
        )
        .row('outcome', `${outcome.kind} ${outcome.value}`)
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
