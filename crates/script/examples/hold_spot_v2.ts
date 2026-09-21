type NativeApi = import('../host-js/index.d.ts').NativeApi;

/**
 * Headed File witness: field observation only. Range hold with dest a
 * neighbouring tile (Chebyshev 2–6). One holdNext must be status or walk
 * to dest — not walk-to / npc / Attack. Already-on-dest is not PASS.
 */
export const apiVersion = 2;

const STOP_OK = 'hold spot qualification complete';
const RECEIPT_PREFIX = 'hold-spot-receipt:';
const WALK_SCENERY = 0x100;
const MIN_CHEB = 2;
const MAX_CHEB = 6;

type Tile = { x: number; z: number; level: number };

let token: number | null = null;
let dest: Tile | null = null;

function chebyshev(a: Tile, b: Tile): number {
    return Math.max(Math.abs(a.x - b.x), Math.abs(a.z - b.z));
}

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

function pickDest(here: Tile, c: NativeApi['snapshot']['collision']): Tile | null {
    for (let r = MIN_CHEB; r <= MAX_CHEB; r++) {
        for (let dx = -r; dx <= r; dx++) {
            for (let dz = -r; dz <= r; dz++) {
                if (Math.max(Math.abs(dx), Math.abs(dz)) !== r) continue;
                const cand = { x: here.x + dx, z: here.z + dz, level: here.level };
                if (cand.x === here.x && cand.z === here.z) continue;
                if (c.available) {
                    const flag = flagAt(c, cand.x, cand.z);
                    if (flag === undefined) continue;
                    if ((flag & WALK_SCENERY) !== 0) continue;
                }
                return cand;
            }
        }
    }
    return null;
}

function forbiddenKind(kind: string | undefined): boolean {
    if (typeof kind !== 'string') return false;
    const k = kind.toLowerCase();
    return k === 'walk-to' || k === 'npc' || k === 'attack';
}

export function tick(api: NativeApi): void {
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
        dest = pickDest(here, c);
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
    const projection = {
        died: false,
        targetIdx: null,
        hpFraction: 1,
        panicHp: 0.1,
        retreatHp: 0.2,
        hasFood: true,
        needEat: false,
        style: 'range',
        safespotIndex: 0,
        buryBones: false,
        boneName: 'Bones',
        hasVlog: false,
        hasArmSpecial: false,
        hasShieldReady: false,
        key: 'hold-spot',
        target: 'hold',
        alsoHunt: [],
        safespots: [{ x: dest.x, z: dest.z, level: dest.level }],
        meleeAnchor: { x: here.x, z: here.z, level: here.level },
        boxes: [siteBox],
        fireAtRange: false,
        rangedThreat: false,
    };

    if (token == null) {
        const began = api.holdBegin();
        if (!began.ok) {
            throw new Error(began.error);
        }
        token = began.value.token;
    }
    const validated = api.holdValidate({ token, ...projection });
    if (!validated.ok) {
        throw new Error(validated.error);
    }
    if (!validated.value) {
        return;
    }
    const step = api.holdNext({ token, ...projection });
    if (!step.ok) {
        throw new Error(step.error);
    }
    const kind = 'kind' in step ? step.kind : undefined;
    if (forbiddenKind(kind)) {
        throw new Error('hold spot witness must not emit walk-to / npc / Attack');
    }
    if (kind !== 'status' && kind !== 'walk') {
        return;
    }
    if (kind === 'walk') {
        const walk = step as { x?: number; z?: number; level?: number };
        if (walk.x !== dest.x || walk.z !== dest.z || walk.level !== dest.level) {
            throw new Error('holdNext walk must target dest');
        }
    }

    const receipt = {
        here: { x: here.x, z: here.z, level: here.level },
        dest: { x: dest.x, z: dest.z, level: dest.level },
        kind,
    };
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint
        .begin()
        .title('hold spot v2')
        .row(line)
        .row('here', `${here.x},${here.z},${here.level}`, 'dest', `${dest.x},${dest.z},${dest.level}`)
        .row('kind', String(kind))
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
