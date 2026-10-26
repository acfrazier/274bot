type NativeApi = import('../host-js/index.d.ts').NativeApi;
type HuntHooks = import('../host-js/index.d.ts').HuntHooks;
type HuntSite = import('../host-js/index.d.ts').HuntSite;

/**
 * Headed File witness: one retreat hop to a neighbouring tile (Chebyshev
 * 2–6), never meleeAnchor. begin + validate + one awaited retreatRun hops
 * with walk-to to dest (not a world walk / npc / Attack). retreatRun
 * settles `done` with `null` even when its stepper gives up, so the tile
 * after the run is the proof: anything but `done` on dest throws. The
 * receipt names the Start tile, the tile after the run, dest and the
 * outcome; the gate checks them against the host's tile and walk-to.
 */
export const apiVersion = 2;

const STOP_OK = 'retreat spot qualification complete';
const RECEIPT_PREFIX = 'retreat-spot-receipt:';
const MIN_CHEB = 2;
const MAX_CHEB = 6;

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
    if (chebyshev(here, dest) < MIN_CHEB || chebyshev(here, dest) > MAX_CHEB) {
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
        key: 'retreat-spot',
        target: 'retreat',
        alsoHunt: [],
        safespots: [{ x: dest.x, z: dest.z, level: dest.level }],
        meleeAnchor,
        boxes: [siteBox],
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
        style: () => 'melee',
        safespotIndex: () => 0,
        buryBones: () => false,
        boneName: () => 'Bones',
    };

    if (token == null) {
        const began = api.retreatBegin(site);
        if (!began.ok) {
            throw new Error(began.error);
        }
        token = began.value.token;
    }
    const validated = api.retreatValidate({ token }, hooks);
    if (!validated.ok) {
        throw new Error(validated.error);
    }
    if (!validated.value) {
        return;
    }

    running = true;
    const from = { x: here.x, z: here.z, level: here.level };
    const outcome = await api.retreatRun({ token }, hooks);
    const after = api.snapshot.here;
    if (outcome.kind !== 'done' || !after || !sameTile(after, dest)) {
        throw new Error(
            `retreatRun did not reach ${dest.x},${dest.z}: ${JSON.stringify(outcome)} at ${JSON.stringify(after)}`,
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
        .title('retreat spot v2')
        .row(line)
        .row('from', `${from.x},${from.z},${from.level}`, 'dest', `${dest.x},${dest.z},${dest.level}`)
        .row('outcome', outcome.kind)
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
