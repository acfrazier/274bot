type NativeApi = import('../host-js/index.d.ts').NativeApi;

/**
 * Headed File witness: field observation only. Dest is Chebyshev 13–20,
 * never a 2-tile Hold walk-back and never meleeAnchor. One walkspotNext
 * must be `walk` toward dest. The isolate omits radius; the shim queues
 * that walk at radius 0. Do not ack it — acking arms the 120s walk.
 */
export const apiVersion = 2;

const STOP_OK = 'walk spot qualification complete';
const RECEIPT_PREFIX = 'walk-spot-receipt:';
const WALK_SCENERY = 0x100;
const MIN_CHEB = 13;
const MAX_CHEB = 20;

type Tile = { x: number; z: number; level: number };

let token: number | null = null;
let dest: Tile | null = null;

function chebyshev(a: Tile, b: Tile): number {
    return Math.max(Math.abs(a.x - b.x), Math.abs(a.z - b.z));
}

function sameTile(a: Tile, b: Tile): boolean {
    return a.x === b.x && a.z === b.z && a.level === b.level;
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
                if (sameTile(cand, here)) continue;
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
    return (
        k === 'walk-to' ||
        k === 'walk-near' ||
        k === 'npc' ||
        k === 'attack' ||
        k === 'set-safespot'
    );
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

    if (dest && sameTile(dest, here)) {
        dest = null;
    }
    if (!dest) {
        dest = pickDest(here, c);
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
    const projection = {
        died: false,
        targetIdx: null,
        hpFraction: 1,
        panicHp: 0.1,
        retreatHp: 0.2,
        hasFood: false,
        needEat: false,
        style: 'range',
        safespotIndex: 0,
        buryBones: false,
        boneName: 'Bones',
        hasVlog: false,
        hasArmSpecial: false,
        hasShieldReady: false,
        key: 'walk-spot',
        target: 'walk',
        alsoHunt: [],
        safespots: [{ x: dest.x, z: dest.z, level: dest.level }],
        meleeAnchor,
        boxes: [siteBox],
        fireAtRange: false,
        rangedThreat: false,
        approach: [],
    };

    if (token == null) {
        const began = api.walkspotBegin();
        if (!began.ok) {
            throw new Error(began.error);
        }
        token = began.value.token;
    }
    const validated = api.walkspotValidate({ token, ...projection });
    if (!validated.ok) {
        throw new Error(validated.error);
    }
    if (!validated.value) {
        return;
    }

    for (let i = 0; i < 4; i++) {
        const step = api.walkspotNext({ token, ...projection });
        if (!step.ok) {
            throw new Error(step.error);
        }
        const kind = 'kind' in step ? step.kind : undefined;
        if (forbiddenKind(kind)) {
            throw new Error(
                'walk spot witness must not emit walk-to / walk-near / npc / Attack / set-safespot',
            );
        }
        if (kind === 'status') {
            continue;
        }
        if (kind !== 'walk') {
            throw new Error(`walkspotNext expected walk radius 0, got ${String(kind)}`);
        }
        const walk = step as { x?: number; z?: number; level?: number; radius?: number };
        if (walk.x !== dest.x || walk.z !== dest.z || walk.level !== dest.level) {
            throw new Error('walkspotNext walk must target dest');
        }
        if (sameTile({ x: walk.x, z: walk.z, level: walk.level ?? here.level }, meleeAnchor)) {
            throw new Error('walkspotNext walk must not target meleeAnchor');
        }
        if (typeof walk.radius === 'number' && walk.radius !== 0) {
            throw new Error('walkspotNext walk must be radius 0');
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
            .title('walk spot v2')
            .row(line)
            .row('here', `${here.x},${here.z},${here.level}`, 'dest', `${dest.x},${dest.z},${dest.level}`)
            .row('kind', String(kind))
            .row('result', STOP_OK)
            .end();
        api.stop(STOP_OK);
        return;
    }
    throw new Error('walkspotNext did not emit walk radius 0');
}
