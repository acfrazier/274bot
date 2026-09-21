type NativeApi = import('../host-js/index.d.ts').NativeApi;

/**
 * File-card v2 consumer: logs one packed NPC row and existing lineOfSight,
 * then stops. Packet-time facts only; size<1 is unavailable.
 */
export const apiVersion = 2;

const STOP_OK = 'actor observation v2 complete';

export function tick(api: NativeApi) {
    const snap = api.snapshot;
    if (!snap.ingame || !snap.here) {
        return;
    }
    const n = (snap.npcs || []).find((row) => row && row.size >= 1);
    if (!n) {
        api.log('actor observation: no NPC size>=1 this tick');
        api.stop(STOP_OK);
        return;
    }
    const los = api.lineOfSight({
        from: snap.here,
        to: { x: n.nx, z: n.nz, level: n.level },
        size: n.size,
    });
    api.log(
        `npc index=${n.index} name=${n.name} size=${n.size} tile=${n.x},${n.z} network=${n.nx},${n.nz} self=${snap.self_target_kind},${snap.self_target_index} los=${JSON.stringify(los)}`,
    );
    api.stop(STOP_OK);
}
