import { Npc } from '../../api/npcs/Npcs.js';
import { reader } from '../../adapter/ClientAdapter.js';
import { Reachability } from '../../event/webwalk/geometry/Reachability.js';

type NativeApi = import('../host-js/index.d.ts').NativeApi;

/**
 * Headed File witness: one posted NPC row with size>=1, v1 Npc/reader
 * helpers plus existing v1/v2 line-of-sight, then named stop.
 * no-NPC / unready is not complete — keep waiting within the scenario budget.
 */
export const apiVersion = 2;

const STOP_OK = 'actor observation qualification complete';
const RECEIPT_PREFIX = 'actor-receipt:';

export function tick(api: NativeApi): void {
    const snap = api.snapshot;
    if (!snap.ingame || !snap.here) {
        return;
    }
    const c = snap.collision;
    if (!c.available) {
        return;
    }
    const n = (snap.npcs || []).find((row) => row && row.size >= 1);
    if (!n) {
        return;
    }

    const wrapped = new Npc(n);
    const origin = wrapped.networkOrigin();
    const tile = wrapped.tile();
    const self = reader.selfTarget();
    if (wrapped.size !== n.size || origin.x !== n.nx || origin.z !== n.nz) {
        throw new Error('v1 Npc packed fields disagree with snapshot row');
    }
    if (tile.x !== n.x || tile.z !== n.z || tile.level !== n.level) {
        throw new Error('v1 Npc.tile disagrees with rendered snapshot row');
    }
    if (self.kind !== snap.self_target_kind || self.index !== snap.self_target_index) {
        throw new Error('reader.selfTarget disagrees with snapshot self_target_*');
    }

    const to = { x: origin.x, z: origin.z, level: n.level };
    const v2 = api.lineOfSight({ from: snap.here, to, size: wrapped.size });
    if (!v2.ok) {
        throw new Error(v2.error);
    }
    const v1 = Reachability.lineOfSight(snap.here, to, wrapped.size);
    if (v1 !== v2.value) {
        throw new Error('v1 Reachability.lineOfSight must agree with api.lineOfSight');
    }

    const receipt = {
        identity: {
            base_x: c.base_x,
            base_z: c.base_z,
            level: c.level,
            width: c.width,
            height: c.height,
        },
        here: { x: snap.here.x, z: snap.here.z, level: snap.here.level },
        npc: {
            index: n.index,
            name: n.name,
            size: n.size,
            tile: { x: n.x, z: n.z },
            network: { x: n.nx, z: n.nz },
            level: n.level,
        },
        packed: { size: wrapped.size, nx: origin.x, nz: origin.z },
        rendered: { x: tile.x, z: tile.z },
        self_target: { kind: self.kind, index: self.index },
        los: { v2: v2.value, v1 },
    };
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(
        `npc index=${n.index} name=${n.name} size=${n.size} tile=${n.x},${n.z} network=${n.nx},${n.nz} level=${n.level} self=${self.kind},${self.index} los=${v2.value}`,
    );
    api.log(line);
    api.paint
        .begin()
        .title('actor observation v2')
        .row(line)
        .row('npc', n.index, n.name ?? '', `size=${n.size}`)
        .row('tile', `${n.x},${n.z}`, 'network', `${n.nx},${n.nz}`, 'level', n.level)
        .row('self', self.kind, self.index, 'los', v2.value)
        .row('result', STOP_OK)
        .end();
    (globalThis as { __actorExample?: unknown }).__actorExample = {
        ok: true,
        receipt,
    };
    api.stop(STOP_OK);
}
