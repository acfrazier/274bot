type NativeApi = import('../host-js/index.d.ts').NativeApi;

/**
 * Headed File witness: field observation of the gateless first effect.
 * Not a lair entry and not inArea success. Curated box does not contain
 * here. One approach tile inside that box, same level, Chebyshev 8–16
 * (greater than the skip of 1, not Hold's 2-tile walk-back, not Walk's
 * > 12 gate). The picker scans that Chebyshev ring and may choose a
 * diagonal. A missing collision map does not reject the tile. One
 * enterNext must be `walk` radius 0 toward that tile.
 * The isolate omits radius; the shim would queue it at radius 0 with
 * allow_teleports, allow_wilderness, and allow_bank_fetch all false.
 * Do not ack the walk — acking arms the 120s bound. Do not wait out the
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

function forbiddenKind(kind: string | undefined): boolean {
    if (typeof kind !== 'string') return false;
    const k = kind.toLowerCase();
    return (
        k === 'walk-to' ||
        k === 'walk-near' ||
        k === 'npc' ||
        k === 'loc' ||
        k === 'use-on' ||
        k === 'answer' ||
        k === 'bank-open' ||
        k === 'attack' ||
        k === 'kbd' ||
        k === 'kbd-lair'
    );
}

export function tick(api: NativeApi): void {
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

    const projection = {
        parked: false,
        shieldReady: true,
        hpFraction: 1,
        panicHp: 0.2,
        key: SITE_KEY,
        boxes: [box],
        approach: [{ x: approach.x, z: approach.z, level: approach.level }],
        talkGate: null,
        feeGate: null,
        gate: null,
        keyItem: null,
    };
    if (projection.key === 'kbd-lair') {
        throw new Error('enter lair witness must not use kbd-lair');
    }

    if (token == null) {
        const began = api.enterBegin();
        if (!began.ok) {
            throw new Error(began.error);
        }
        token = began.value.token;
    }
    const validated = api.enterValidate({ token, ...projection });
    if (!validated.ok) {
        throw new Error(validated.error);
    }
    if (!validated.value) {
        return;
    }

    for (let i = 0; i < 4; i++) {
        const step = api.enterNext({ token, ...projection });
        if (!step.ok) {
            throw new Error(step.error);
        }
        const kind = 'kind' in step ? step.kind : undefined;
        if (kind === 'yield') {
            throw new Error('enter lair witness must not claim inArea');
        }
        if (forbiddenKind(kind)) {
            throw new Error(
                'enter lair witness must not emit walk-near / walk-to / npc / loc / use-on / answer / bank-open / Attack / KBD',
            );
        }
        if (kind === 'status' || kind === 'log' || kind === 'wait') {
            continue;
        }
        if (kind !== 'walk') {
            throw new Error(`enterNext expected walk radius 0, got ${String(kind)}`);
        }
        const walk = step as {
            x?: number;
            z?: number;
            level?: number;
            radius?: number;
            allow_teleports?: boolean;
            allow_wilderness?: boolean;
            allow_bank_fetch?: boolean;
        };
        if (walk.x !== approach.x || walk.z !== approach.z || walk.level !== approach.level) {
            throw new Error('enterNext walk must target the approach tile');
        }
        if (typeof walk.radius === 'number' && walk.radius !== 0) {
            throw new Error('enterNext walk must be radius 0');
        }
        if (
            walk.allow_teleports === true ||
            walk.allow_wilderness === true ||
            walk.allow_bank_fetch === true
        ) {
            throw new Error('enterNext walk must keep teleports, wilderness, and bank fetch off');
        }

        const receipt = {
            here: { x: here.x, z: here.z, level: here.level },
            approach: { x: approach.x, z: approach.z, level: approach.level },
            kind,
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
            .row('kind', String(kind), 'discriminator', 'gateless')
            .row('result', STOP_OK)
            .end();
        api.stop(STOP_OK);
        return;
    }
    throw new Error('enterNext did not emit walk radius 0');
}
