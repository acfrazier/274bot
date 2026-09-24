type NativeApi = import('../host-js/index.d.ts').NativeApi;
type HuntSite = import('../host-js/index.d.ts').HuntSite;

/**
 * Headed File witness: one trip through Velrak's cell. The scenario seeds
 * the account in the Taverley dungeon with a jail key, far from the cell
 * door. One awaited cellRun follows frozen `fetchFromVelrak`
 * (`supply.ts:942-965`): walk near the jail door (2931,9690,0) at radius 1,
 * stand on it, use the jail key on the door, talk to Velrak for the dusty
 * key, then open the door from inside. The run settles `true` only when the
 * dusty key is held outside the cell; anything else throws. The receipt
 * names the Start tile, the tile after the run, the door, the site key and
 * the outcome; the gate checks the host's walk, unlock, talk, open, tile
 * and inventory.
 */
export const apiVersion = 2;

const STOP_OK = 'cell qualification complete';
const RECEIPT_PREFIX = 'cell-receipt:';
const SITE_KEY = 'taverley-blue';
const JAIL_DOOR = { x: 2931, z: 9690, level: 0 };
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

    const site: HuntSite = {
        key: SITE_KEY,
        keyItem: { name: 'Dusty key', id: 1590 },
        boxes: [LAIR],
    };

    running = true;
    const from = { x: here.x, z: here.z, level: here.level };
    const outcome = await api.cellRun(site, {});
    const after = api.snapshot.here;
    if (outcome.kind !== 'done' || outcome.value !== true || !after || contains(CELL, after)) {
        throw new Error(`cellRun did not bring the dusty key out: ${JSON.stringify(outcome)} at ${JSON.stringify(after)}`);
    }

    const receipt = {
        outcome,
        from,
        here: { x: after.x, z: after.z, level: after.level },
        dest: JAIL_DOOR,
        key: SITE_KEY,
    };
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint
        .begin()
        .title('cell v2')
        .row(line)
        .row('from', `${from.x},${from.z},${from.level}`, 'here', `${after.x},${after.z},${after.level}`)
        .row('outcome', `${outcome.kind} ${outcome.value}`)
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
