type NativeApi = import('../host-js/index.d.ts').NativeApi;
type HuntSite = import('../host-js/index.d.ts').HuntSite;

/**
 * Headed File witness: one Jailer leg. The scenario seeds the account in the
 * Taverley dungeon, far from the prison corridor and outside the jail cell.
 * One awaited keyRun: the host walks near the corridor (2931,9690,0) at
 * radius 1, attacks the Jailer and takes the jail key he drops. The run
 * settles `true` only when the jail key is held; anything else throws. The
 * receipt names the Start tile, the tile after the run, the corridor, the
 * site key and the outcome; the gate checks the host's walk, attack, tile
 * and inventory.
 */
export const apiVersion = 2;

const STOP_OK = 'acquire key qualification complete';
const RECEIPT_PREFIX = 'acquire-key-receipt:';
const SITE_KEY = 'taverley-blue';
const CORRIDOR = { x: 2931, z: 9690, level: 0 };
const CELL = { minX: 2928, maxX: 2934, minZ: 9683, maxZ: 9689, level: 0 };
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

    // The key item makes the run fetch: without one it settles at once.
    const site: HuntSite = {
        key: SITE_KEY,
        keyItem: { name: 'Dusty key', id: 1590 },
        boxes: [LAIR],
    };

    running = true;
    const from = { x: here.x, z: here.z, level: here.level };
    const outcome = await api.keyRun(site, {});
    const after = api.snapshot.here;
    if (outcome.kind !== 'done' || outcome.value !== true || !after) {
        throw new Error(`keyRun did not take the jail key: ${JSON.stringify(outcome)} at ${JSON.stringify(after)}`);
    }

    const receipt = {
        outcome,
        from,
        here: { x: after.x, z: after.z, level: after.level },
        dest: CORRIDOR,
        key: SITE_KEY,
    };
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint
        .begin()
        .title('acquire key v2')
        .row(line)
        .row('from', `${from.x},${from.z},${from.level}`, 'here', `${after.x},${after.z},${after.level}`)
        .row('outcome', `${outcome.kind} ${outcome.value}`)
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
