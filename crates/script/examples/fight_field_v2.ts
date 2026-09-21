import { Npc } from '../../api/npcs/Npcs.js';
import { Reachability } from '../../event/webwalk/geometry/Reachability.js';

type NativeApi = import('../host-js/index.d.ts').NativeApi;

/**
 * Headed File witness: field observation only. fightValidate plus at most
 * one fightNext must not emit npc / Attack.
 */
export const apiVersion = 2;

const STOP_OK = 'fight field qualification complete';
const RECEIPT_PREFIX = 'fight-field-receipt:';

let token: number | null = null;

export function tick(api: NativeApi): void {
    const snap = api.snapshot;
    if (!snap.ingame || !snap.here) {
        return;
    }
    const c = snap.collision;
    if (!c.available) {
        return;
    }
    let n: (typeof snap.npcs)[number] | undefined;
    for (const row of snap.npcs || []) {
        if (!row || row.size < 1) {
            continue;
        }
        if (
            !n ||
            row.distance < n.distance ||
            (row.distance === n.distance && row.index < n.index)
        ) {
            n = row;
        }
    }
    if (!n) {
        return;
    }

    const wrapped = new Npc(n);
    const origin = wrapped.networkOrigin();
    const tile = wrapped.tile();
    const here = snap.here;
    const siteBox = {
        minX: Math.min(here.x, origin.x, tile.x) - 8,
        maxX: Math.max(here.x, origin.x, tile.x) + 8,
        minZ: Math.min(here.z, origin.z, tile.z) - 8,
        maxZ: Math.max(here.z, origin.z, tile.z) + 8,
        level: here.level,
    };
    const projection = {
        died: false,
        targetIdx: null,
        hpFraction: 1,
        panicHp: 0.1,
        retreatHp: 0.2,
        hasFood: true,
        needEat: false,
        style: 'range',
        safespotIndex: 0,
        buryBones: false,
        boneName: 'Bones',
        hasVlog: false,
        hasArmSpecial: false,
        hasShieldReady: false,
        key: 'fight-field',
        target: n.name || 'npc',
        alsoHunt: [],
        safespots: [{ x: here.x, z: here.z, level: here.level }],
        meleeAnchor: { x: here.x, z: here.z, level: here.level },
        boxes: [siteBox],
        fireAtRange: false,
        rangedThreat: false,
    };

    if (token == null) {
        const began = api.fightBegin();
        if (!began.ok) {
            throw new Error(began.error);
        }
        token = began.value.token;
    }
    const validated = api.fightValidate({ token, ...projection });
    if (!validated.ok) {
        throw new Error(validated.error);
    }
    if (!validated.value) {
        return;
    }
    const step = api.fightNext({ token, ...projection });
    if (!step.ok) {
        throw new Error(step.error);
    }
    const kind = 'kind' in step ? step.kind : undefined;
    if (kind === 'npc' || kind === 'Attack' || kind === 'attack') {
        throw new Error('fight field witness must not emit npc / Attack');
    }

    const toNet = { x: origin.x, z: origin.z, level: n.level };
    const toTile = { x: tile.x, z: tile.z, level: n.level };
    const losNetwork = api.lineOfSight({ from: here, to: toNet, size: wrapped.size });
    const losTile = api.lineOfSight({ from: here, to: toTile, size: wrapped.size });
    if (!losNetwork.ok) {
        throw new Error(losNetwork.error);
    }
    if (!losTile.ok) {
        throw new Error(losTile.error);
    }
    const v1 = Reachability.lineOfSight(here, toNet, wrapped.size);
    if (v1 !== losNetwork.value) {
        throw new Error('v1 Reachability.lineOfSight must agree with api.lineOfSight');
    }
    if (origin.x !== n.nx || origin.z !== n.nz) {
        throw new Error('networkOrigin must equal packed nx,nz');
    }

    const receipt: {
        index: number;
        size: number;
        tile: { x: number; z: number };
        networkOrigin: { x: number; z: number };
        losNetwork: boolean;
        losTile: boolean;
        kind?: string;
    } = {
        index: n.index,
        size: n.size,
        tile: { x: n.x, z: n.z },
        networkOrigin: { x: origin.x, z: origin.z },
        losNetwork: losNetwork.value,
        losTile: losTile.value,
    };
    if (typeof kind === 'string') {
        receipt.kind = kind;
    }
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint
        .begin()
        .title('fight field v2')
        .row(line)
        .row(`index=${n.index} size=${n.size}`)
        .row('tile', `${n.x},${n.z}`, 'network', `${origin.x},${origin.z}`)
        .row('losNetwork', String(losNetwork.value), 'losTile', String(losTile.value))
        .row('result', STOP_OK)
        .end();
    api.stop(STOP_OK);
}
