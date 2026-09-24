type NativeApi = import('../host-js/index.d.ts').NativeApi;
type HuntSite = import('../host-js/index.d.ts').HuntSite;

/**
 * Headed File witness: field observation of the approach walk.
 * Not an opened bank and not pack completion. Mainland Lumbridge is not
 * within 3 of the Taverley bank tile. The machine emits leave only when
 * here is inside the projected lair boxes. Do not put here in those boxes.
 * One bankRun starts the host walk-near radius 3 with teleports,
 * wilderness, and bank fetch off. Do not wait out the walk. Gate is
 * ingame and here only. The v2 snapshot does not publish a scene field.
 */
export const apiVersion = 2;

const STOP_OK = 'bank qualification complete';
const RECEIPT_PREFIX = 'bank-receipt:';
const BANK = { x: 2946, z: 3369, level: 0 };
const LAIR = { minX: 40, maxX: 60, minZ: 40, maxZ: 60, level: 0 };

type Tile = { x: number; z: number; level: number };
type Box = { minX: number; maxX: number; minZ: number; maxZ: number; level: number };

let running = false;

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

export async function tick(api: NativeApi): Promise<void> {
    if (running) {
        return;
    }
    const snap = api.snapshot;
    if (!snap.ingame || !snap.here) {
        return;
    }
    const here = snap.here;
    if (contains(LAIR, here) || chebyshev(here, BANK) <= 3) {
        return;
    }

    const site: HuntSite = {
        key: 'bank',
        bank: BANK,
        keyItem: null,
        boxes: [LAIR],
    };
    if (site.boxes!.some((box) => contains(box, here))) {
        return;
    }

    running = true;
    const run = api.bankRun(site, {}, {});
    void run;

    const flags = {
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
    };
    const receipt = {
        here: { x: here.x, z: here.z, level: here.level },
        dest: { x: BANK.x, z: BANK.z, level: BANK.level },
        radius: 3,
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
        .row('discriminator', 'approach')
        .row('radius', '3')
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
