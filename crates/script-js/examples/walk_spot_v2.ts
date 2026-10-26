type NativeApi = import('../host-js/index.d.ts').NativeApi;
type HuntHooks = import('../host-js/index.d.ts').HuntHooks;
type HuntSite = import('../host-js/index.d.ts').HuntSite;

/**
 * Headed File witness: one walk to a tile Chebyshev 13–20 away, never a
 * 2-tile Hold walk-back and never meleeAnchor. begin + validate + one
 * awaited walkspotRun walks the world to dest at radius 0; the host owns
 * the walk, so do not invent allow-flags. walkspotRun settles `done` with
 * `null` even when its stepper gives up, so the tile after the run is the
 * proof: anything but `done` on dest throws. The receipt names the Start
 * tile, the tile after the run, dest and the outcome.
 */
export const apiVersion = 2;

const STOP_OK = 'walk spot qualification complete';
const RECEIPT_PREFIX = 'walk-spot-receipt:';
const MIN_CHEB = 13;
const MAX_CHEB = 20;

type Tile = { x: number; z: number; level: number };

let token: number | null = null;
let dest: Tile | null = null;
let running = false;

function chebyshev(a: Tile, b: Tile): number {
    return Math.max(Math.abs(a.x - b.x), Math.abs(a.z - b.z));
}

function sameTile(a: Tile, b: Tile): boolean {
    return a.x === b.x && a.z === b.z && a.level === b.level;
}

function tileWalkable(api: NativeApi, tile: Tile): boolean {
    const r = api.walkable({ tile });
    if (!r.ok) return true;
    return r.value === true;
}

function pickDest(here: Tile, api: NativeApi): Tile | null {
    for (let r = MIN_CHEB; r <= MAX_CHEB; r++) {
        for (let dx = -r; dx <= r; dx++) {
            for (let dz = -r; dz <= r; dz++) {
                if (Math.max(Math.abs(dx), Math.abs(dz)) !== r) continue;
                const cand = { x: here.x + dx, z: here.z + dz, level: here.level };
                if (sameTile(cand, here)) continue;
                if (!tileWalkable(api, cand)) continue;
                return cand;
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
    const c = snap.collision;
    if (!c.available) {
        return;
    }

    if (dest && sameTile(dest, here)) {
        dest = null;
    }
    if (!dest) {
        dest = pickDest(here, api);
    }
    if (!dest) {
        return;
    }
    const gap = chebyshev(here, dest);
    if (gap < MIN_CHEB || gap > MAX_CHEB) {
        dest = null;
        return;
    }

    const meleeAnchor = { x: here.x, z: here.z, level: here.level };
    if (sameTile(dest, meleeAnchor)) {
        dest = null;
        return;
    }

    const siteBox = {
        minX: Math.min(here.x, dest.x),
        maxX: Math.max(here.x, dest.x),
        minZ: Math.min(here.z, dest.z),
        maxZ: Math.max(here.z, dest.z),
        level: here.level,
    };
    const site: HuntSite = {
        key: 'walk-spot',
        target: 'walk',
        alsoHunt: [],
        safespots: [{ x: dest.x, z: dest.z, level: dest.level }],
        meleeAnchor,
        boxes: [siteBox],
        approach: [],
        fireAtRange: false,
        rangedThreat: false,
    };
    const hooks: HuntHooks = {
        died: () => false,
        hpFraction: () => 1,
        panicHp: () => 0.1,
        retreatHp: () => 0.2,
        hasFood: () => false,
        needEat: () => false,
        style: () => 'range',
        safespotIndex: () => 0,
        buryBones: () => false,
        boneName: () => 'Bones',
    };

    if (token == null) {
        const began = api.walkspotBegin(site);
        if (!began.ok) {
            throw new Error(began.error);
        }
        token = began.value.token;
    }
    const validated = api.walkspotValidate({ token }, hooks);
    if (!validated.ok) {
        throw new Error(validated.error);
    }
    if (!validated.value) {
        return;
    }

    running = true;
    const from = { x: here.x, z: here.z, level: here.level };
    const outcome = await api.walkspotRun({ token }, hooks);
    const after = api.snapshot.here;
    if (outcome.kind !== 'done' || !after || !sameTile(after, dest)) {
        throw new Error(
            `walkspotRun did not reach ${dest.x},${dest.z}: ${JSON.stringify(outcome)} at ${JSON.stringify(after)}`,
        );
    }

    const receipt = {
        outcome,
        from,
        here: { x: after.x, z: after.z, level: after.level },
        dest: { x: dest.x, z: dest.z, level: dest.level },
    };
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint
        .begin()
        .title('walk spot v2')
        .row(line)
        .row('from', `${from.x},${from.z},${from.level}`, 'dest', `${dest.x},${dest.z},${dest.level}`)
        .row('outcome', outcome.kind)
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
