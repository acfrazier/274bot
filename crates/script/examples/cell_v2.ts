type NativeApi = import('../host-js/index.d.ts').NativeApi;
type HuntSite = import('../host-js/index.d.ts').HuntSite;

/**
 * Headed File witness: field observation of the nested key run.
 * Not cell entry, not Velrak, and not a dusty key. Mainland Lumbridge
 * has no jail key. Do not invent a jail key, a door, or Velrak.
 * One cellRun starts the nested key family. Do not wait out the jail.
 * Gate is ingame and here only. The v2 snapshot does not publish a
 * scene field.
 */
export const apiVersion = 2;

const STOP_OK = 'cell qualification complete';
const RECEIPT_PREFIX = 'cell-receipt:';
const SITE_KEY = 'taverley-blue';
const CELL = { minX: 2928, maxX: 2934, minZ: 9683, maxZ: 9689, level: 0 };
const LAIR = { minX: 40, maxX: 60, minZ: 40, maxZ: 60, level: 0 };

type Tile = { x: number; z: number; level: number };
type Box = { minX: number; maxX: number; minZ: number; maxZ: number; level: number };

let running = false;

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

export async function tick(api: NativeApi): Promise<void> {
    if (running) {
        return;
    }
    const snap = api.snapshot;
    if (!snap.ingame || !snap.here) {
        return;
    }
    const here = snap.here;
    if (contains(CELL, here) || contains(LAIR, here)) {
        return;
    }

    const site: HuntSite = {
        key: SITE_KEY,
        keyItem: { name: 'key', id: 1 },
        boxes: [LAIR],
    };
    if (site.key !== SITE_KEY) {
        return;
    }
    if (site.boxes!.some((box) => contains(box, here) || sameBox(box, CELL))) {
        return;
    }

    running = true;
    const run = api.cellRun(site, {});
    void run;

    const receipt = {
        here: { x: here.x, z: here.z, level: here.level },
        discriminator: 'key-call',
    };
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint
        .begin()
        .title('cell v2')
        .row(line)
        .row('here', `${here.x},${here.z},${here.level}`)
        .row('discriminator', 'key-call')
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
