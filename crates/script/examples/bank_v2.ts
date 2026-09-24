type NativeApi = import('../host-js/index.d.ts').NativeApi;
type HuntSite = import('../host-js/index.d.ts').HuntSite;

/**
 * Headed File witness: one bank trip. The scenario seeds the account near
 * Falador, outside the approach radius of the site bank (2946,3369,0) and
 * outside the projected lair box, so the run never takes the leave leg.
 * One awaited bankRun: the host walks near the bank at radius 3 with
 * teleports, wilderness and bank fetch off, opens it, deposits the pack and
 * closes it. No food is withdrawn and the site has no key. The run settles
 * `true` only when the trip restocked; anything else throws. The receipt
 * names the Start tile, the tile after the run, the bank, the site key and
 * the outcome; the gate checks the host's walk and tile.
 */
export const apiVersion = 2;

const STOP_OK = 'bank qualification complete';
const RECEIPT_PREFIX = 'bank-receipt:';
const SITE_KEY = 'bank';
const BANK = { x: 2946, z: 3369, level: 0 };
const RADIUS = 3;
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
    if (contains(LAIR, here) || chebyshev(here, BANK) <= RADIUS) {
        return;
    }

    const site: HuntSite = {
        key: SITE_KEY,
        bank: BANK,
        keyItem: null,
        boxes: [LAIR],
    };

    running = true;
    const from = { x: here.x, z: here.z, level: here.level };
    const outcome = await api.bankRun(site, { withdrawFood: false }, {});
    const after = api.snapshot.here;
    if (outcome.kind !== 'done' || outcome.value !== true || !after || chebyshev(after, BANK) > RADIUS) {
        throw new Error(`bankRun did not restock: ${JSON.stringify(outcome)} at ${JSON.stringify(after)}`);
    }

    const receipt = {
        outcome,
        from,
        here: { x: after.x, z: after.z, level: after.level },
        dest: BANK,
        key: SITE_KEY,
    };
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint
        .begin()
        .title('bank v2')
        .row(line)
        .row('from', `${from.x},${from.z},${from.level}`, 'here', `${after.x},${after.z},${after.level}`)
        .row('outcome', `${outcome.kind} ${outcome.value}`)
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
