type NativeApi = import('../host-js/index.d.ts').NativeApi;

/**
 * Headed File witness: field observation of the key-call first effect.
 * Not cell entry, not Velrak, and not a dusty key. Mainland Lumbridge
 * has no jail key. Do not invent a jail key, a door, or Velrak.
 * Call cellBegin / cellNext directly. Do not call the ack-and-wait
 * helper. Do not reply. Do not queue. Gate is ingame and here only.
 * The v2 snapshot does not publish a scene field.
 */
export const apiVersion = 2;

const STOP_OK = 'cell qualification complete';
const RECEIPT_PREFIX = 'cell-receipt:';
const SITE_KEY = 'taverley-blue';
const CELL = { minX: 2928, maxX: 2934, minZ: 9683, maxZ: 9689, level: 0 };
const LAIR = { minX: 40, maxX: 60, minZ: 40, maxZ: 60, level: 0 };

type Tile = { x: number; z: number; level: number };
type Box = { minX: number; maxX: number; minZ: number; maxZ: number; level: number };

let token: number | null = null;

function sameBox(a: Box, b: Box): boolean {
    return (
        a.minX === b.minX &&
        a.maxX === b.maxX &&
        a.minZ === b.minZ &&
        a.maxZ === b.maxZ &&
        a.level === b.level
    );
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

function forbiddenKind(kind: string | undefined): boolean {
    if (typeof kind !== 'string') return false;
    const k = kind.toLowerCase();
    return (
        k === 'walk' ||
        k === 'walk-near' ||
        k === 'walk-to' ||
        k === 'use-on' ||
        k === 'npc' ||
        k === 'loc' ||
        k === 'leave' ||
        k === 'yield' ||
        k === 'aborted'
    );
}

export function tick(api: NativeApi): void {
    const snap = api.snapshot;
    if (!snap.ingame || !snap.here) {
        return;
    }
    const here = snap.here;
    if (contains(CELL, here) || contains(LAIR, here)) {
        return;
    }

    const projection = {
        key: SITE_KEY,
        keyItem: { present: true },
        boxes: [LAIR],
    };
    if (projection.key !== SITE_KEY) {
        return;
    }
    if (projection.boxes.some((box) => contains(box, here) || sameBox(box, CELL))) {
        return;
    }

    if (token == null) {
        const began = api.cellBegin();
        if (!began.ok) {
            throw new Error(began.error);
        }
        token = began.value.token;
    }

    const step = api.cellNext({ token, ...projection });
    if (!step.ok) {
        throw new Error(step.error);
    }
    const kind = 'kind' in step ? step.kind : undefined;
    if (forbiddenKind(kind)) {
        throw new Error('cellNext must not emit a walk, use-on, npc, loc, or leave');
    }
    if (kind !== 'key') {
        throw new Error(`cellNext expected key, got ${String(kind)}`);
    }
    const stepped = step as { x?: number; z?: number };
    if (typeof stepped.x === 'number' || typeof stepped.z === 'number') {
        throw new Error('cellNext key-call must not carry a walk tile');
    }

    const receipt = {
        here: { x: here.x, z: here.z, level: here.level },
        kind,
        discriminator: 'key-call',
    };
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint
        .begin()
        .title('cell v2')
        .row(line)
        .row('here', `${here.x},${here.z},${here.level}`)
        .row('kind', String(kind), 'discriminator', 'key-call')
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
