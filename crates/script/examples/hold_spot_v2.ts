type NativeApi = import('../host-js/index.d.ts').NativeApi;
type HuntHooks = import('../host-js/index.d.ts').HuntHooks;
type HuntSite = import('../host-js/index.d.ts').HuntSite;

/**
 * Headed File witness: field observation only. Range hold with dest a
 * neighbouring tile (Chebyshev 2–6). begin + validate + one awaited run
 * walks the world to dest (not walk-to / npc / Attack). Already-on-dest
 * is not PASS.
 */
export const apiVersion = 2;

const STOP_OK = 'hold spot qualification complete';
const RECEIPT_PREFIX = 'hold-spot-receipt:';
const MIN_CHEB = 2;
const MAX_CHEB = 6;

type Tile = { x: number; z: number; level: number };

let token: number | null = null;
let dest: Tile | null = null;
let running = false;

function chebyshev(a: Tile, b: Tile): number {
    return Math.max(Math.abs(a.x - b.x), Math.abs(a.z - b.z));
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
                if (cand.x === here.x && cand.z === here.z) continue;
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

    if (dest && dest.x === here.x && dest.z === here.z && dest.level === here.level) {
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

    const siteBox = {
        minX: Math.min(here.x, dest.x),
        maxX: Math.max(here.x, dest.x),
        minZ: Math.min(here.z, dest.z),
        maxZ: Math.max(here.z, dest.z),
        level: here.level,
    };
    const site: HuntSite = {
        key: 'hold-spot',
        target: 'hold',
        alsoHunt: [],
        safespots: [{ x: dest.x, z: dest.z, level: dest.level }],
        meleeAnchor: { x: here.x, z: here.z, level: here.level },
        boxes: [siteBox],
        fireAtRange: false,
        rangedThreat: false,
    };
    const hooks: HuntHooks = {
        died: () => false,
        hpFraction: () => 1,
        panicHp: () => 0.1,
        retreatHp: () => 0.2,
        hasFood: () => true,
        needEat: () => false,
        style: () => 'range',
        safespotIndex: () => 0,
        buryBones: () => false,
        boneName: () => 'Bones',
    };

    if (token == null) {
        const began = api.holdBegin(site);
        if (!began.ok) {
            throw new Error(began.error);
        }
        token = began.value.token;
    }
    const validated = api.holdValidate({ token }, hooks);
    if (!validated.ok) {
        throw new Error(validated.error);
    }
    if (!validated.value) {
        return;
    }

    running = true;
    const outcome = await api.holdRun({ token }, hooks);
    if (outcome.kind !== 'done') {
        throw new Error(`holdRun expected done, got ${outcome.kind}`);
    }

    const receipt = {
        here: { x: here.x, z: here.z, level: here.level },
        dest: { x: dest.x, z: dest.z, level: dest.level },
        outcome,
    };
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint
        .begin()
        .title('hold spot v2')
        .row(line)
        .row('here', `${here.x},${here.z},${here.level}`, 'dest', `${dest.x},${dest.z},${dest.level}`)
        .row('outcome', outcome.kind)
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
