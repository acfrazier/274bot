type NativeApi = import('../host-js/index.d.ts').NativeApi;

/**
 * Headed File witness: field observation of the approach first effect.
 * Not an opened bank and not pack completion. Mainland Lumbridge is not
 * within 3 of the Taverley bank tile. The machine emits leave only when
 * here is inside the projected lair boxes. Do not put here in those boxes.
 * Call bankBegin / bankNext directly. Do not reply. Do not queue. Do not
 * wait out the walk leg. Gate is ingame and here only.
 * The v2 snapshot does not publish a scene field.
 */
export const apiVersion = 2;

const STOP_OK = 'bank qualification complete';
const RECEIPT_PREFIX = 'bank-receipt:';
const BANK = { x: 2946, z: 3369, level: 0 };
const LAIR = { minX: 40, maxX: 60, minZ: 40, maxZ: 60, level: 0 };

type Tile = { x: number; z: number; level: number };
type Box = { minX: number; maxX: number; minZ: number; maxZ: number; level: number };

let token: number | null = null;

function contains(area: Box, tile: Tile): boolean {
    return (
        tile.level === area.level &&
        tile.x >= area.minX &&
        tile.x <= area.maxX &&
        tile.z >= area.minZ &&
        tile.z <= area.maxZ
    );
}

function chebyshev(here: Tile, dest: Tile): number {
    if (here.level !== dest.level) {
        return 1_000_000;
    }
    return Math.max(Math.abs(here.x - dest.x), Math.abs(here.z - dest.z));
}

export function tick(api: NativeApi): void {
    const snap = api.snapshot;
    if (!snap.ingame || !snap.here) {
        return;
    }
    const here = snap.here;
    if (contains(LAIR, here) || chebyshev(here, BANK) <= 3) {
        return;
    }

    const projection = {
        bank: BANK,
        keyItem: null,
        boxes: [LAIR],
    };
    if (projection.boxes.some((box) => contains(box, here))) {
        return;
    }

    if (token == null) {
        const began = api.bankBegin();
        if (!began.ok) {
            throw new Error(began.error);
        }
        token = began.value.token;
    }

    const step = api.bankNext({ token, ...projection });
    if (!step.ok) {
        throw new Error(step.error);
    }
    const kind = 'kind' in step ? step.kind : undefined;
    if (kind !== 'walk-near') {
        throw new Error(`bankNext expected walk-near, got ${String(kind)}`);
    }
    const stepped = step as {
        x?: number;
        z?: number;
        level?: number;
        radius?: number;
        allow_teleports?: boolean;
        allow_wilderness?: boolean;
        allow_bank_fetch?: boolean;
    };
    if (stepped.x !== BANK.x || stepped.z !== BANK.z || stepped.level !== BANK.level) {
        throw new Error('bankNext dest is not the Taverley bank tile');
    }
    if (stepped.radius !== 3) {
        throw new Error(`bankNext expected radius 3, got ${String(stepped.radius)}`);
    }
    const flags = {
        allow_teleports: stepped.allow_teleports === true,
        allow_wilderness: stepped.allow_wilderness === true,
        allow_bank_fetch: stepped.allow_bank_fetch === true,
    };
    if (flags.allow_teleports || flags.allow_wilderness || flags.allow_bank_fetch) {
        throw new Error('bankNext flags must all be false');
    }
    if (chebyshev(here, BANK) <= 3) {
        throw new Error('bankNext approach requires here outside radius 3');
    }

    const receipt = {
        here: { x: here.x, z: here.z, level: here.level },
        dest: { x: stepped.x, z: stepped.z, level: stepped.level },
        kind,
        radius: stepped.radius,
        flags,
        discriminator: 'approach',
    };
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint
        .begin()
        .title('bank v2')
        .row(line)
        .row('here', `${here.x},${here.z},${here.level}`)
        .row('dest', `${BANK.x},${BANK.z},${BANK.level}`)
        .row('kind', String(kind), 'discriminator', 'approach')
        .row('radius', String(stepped.radius))
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
