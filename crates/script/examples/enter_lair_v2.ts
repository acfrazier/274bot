type NativeApi = import('../host-js/index.d.ts').NativeApi;
type HuntHooks = import('../host-js/index.d.ts').HuntHooks;
type HuntSite = import('../host-js/index.d.ts').HuntSite;

/**
 * Headed File witness: field observation of the gateless approach walk.
 * Not a lair entry and not inArea success. Curated box does not contain
 * here. One approach tile inside that box, same level, Chebyshev 8–16
 * (greater than the skip of 1, not Hold's 2-tile walk-back, not Walk's
 * > 12 gate). The picker scans that Chebyshev ring and may choose a
 * diagonal. A missing collision map does not reject the tile.
 * begin + validate starts one run; the host walks radius 0 with
 * teleports, wilderness, and bank fetch off. Do not wait out the
 * approach or the 300s stand. Do not claim inArea.
 * Gate is `ingame` and `here` only. NativeSnapshot does not publish
 * `scene_state`.
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

/** Approach tile only. Chebyshev 8–16 keeps `here` outside this box. */
function approachBox(tile: Tile): Box {
    return {
        minX: tile.x,
        maxX: tile.x,
        minZ: tile.z,
        maxZ: tile.z,
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
    const run = api.enterRun({ token }, hooks);
    void run;

    const receipt = {
        here: { x: here.x, z: here.z, level: here.level },
        approach: { x: approach.x, z: approach.z, level: approach.level },
        discriminator: 'gateless',
        radius: 0,
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
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
            'here',
            `${here.x},${here.z},${here.level}`,
            'approach',
            `${approach.x},${approach.z},${approach.level}`,
        )
        .row('discriminator', 'gateless')
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
