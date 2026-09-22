type NativeApi = import('../host-js/index.d.ts').NativeApi;

/**
 * Headed File witness: field observation of the corridor first effect.
 * Not a key pickup, not a Jailer kill, and not cell entry. Mainland
 * Lumbridge has neither a ground key nor a Jailer. Do not invent either.
 * Call keyBegin / keyNext directly. Do not call the ack-and-wait helper.
 * Do not ack the walk. Do not queue. Do not invent allow-flags. Do not
 * wait out the walk leg. Gate is ingame and here only. The v2 snapshot
 * does not publish a scene field.
 */
export const apiVersion = 2;

const STOP_OK = 'acquire key qualification complete';
const RECEIPT_PREFIX = 'acquire-key-receipt:';
const SITE_KEY = 'acquire-key';
const CORRIDOR = { x: 2931, z: 9690, level: 0 };
const RADIUS = 1;
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
        k === 'npc' ||
        k === 'obj' ||
        k === 'leave' ||
        k === 'yield' ||
        k === 'walk' ||
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
    if (projection.boxes.some((box) => contains(box, here) || sameBox(box, CELL))) {
        return;
    }

    if (token == null) {
        const began = api.keyBegin();
        if (!began.ok) {
            throw new Error(began.error);
        }
        token = began.value.token;
    }

    const step = api.keyNext({ token, ...projection });
    if (!step.ok) {
        throw new Error(step.error);
    }
    const kind = 'kind' in step ? step.kind : undefined;
    if (forbiddenKind(kind)) {
        throw new Error('keyNext must not emit npc / obj / leave / yield / walk');
    }
    if (kind !== 'walk-near') {
        throw new Error(`keyNext expected walk-near radius 1, got ${String(kind)}`);
    }
    const walk = step as {
        x?: number;
        z?: number;
        level?: number;
        radius?: number;
    };
    if (walk.x !== CORRIDOR.x || walk.z !== CORRIDOR.z || walk.level !== CORRIDOR.level) {
        throw new Error('keyNext walk-near must target the corridor');
    }
    if (walk.radius !== RADIUS) {
        throw new Error('keyNext walk-near must be radius 1');
    }

    const receipt = {
        here: { x: here.x, z: here.z, level: here.level },
        dest: { x: CORRIDOR.x, z: CORRIDOR.z, level: CORRIDOR.level },
        kind,
        radius: walk.radius,
        discriminator: 'corridor',
    };
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint
        .begin()
        .title('acquire key v2')
        .row(line)
        .row(
            'here',
            `${here.x},${here.z},${here.level}`,
            'dest',
            `${CORRIDOR.x},${CORRIDOR.z},${CORRIDOR.level}`,
        )
        .row('kind', String(kind), 'discriminator', 'corridor')
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
