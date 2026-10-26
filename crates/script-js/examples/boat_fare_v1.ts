import { Traversal } from '../../api/walking/Traversal.js';
import { ScriptRunner } from '../../runtime/ScriptRunner.js';

/**
 * Qualification-only File adapter for frozen `Traversal.walkTo`'s Karamja
 * boat-fare recovery. Started on Musa Point short of the 30-coin fare, one
 * plain `Traversal.walkTo` to the Port Sarim dock must earn the fare at
 * Luthas's plantation and then take the boat. Not a claimed catalog whale.
 */
const STOP_OK = 'boat fare v1 reached Port Sarim';
const STOP_FAIL = 'boat fare v1 walkTo returned false';
const PORT_SARIM_DOCK = { x: 3029, z: 3217, level: 0 };

export default class BoatFareV1Qualification extends LoopingBot {
    async loop() {
        if (globalThis.__boat_fare_v1_done) {
            return;
        }
        globalThis.__boat_fare_v1_done = true;
        this.log('boat fare v1 phase=walk to the Port Sarim dock');
        const ok = await Traversal.walkTo(PORT_SARIM_DOCK, {
            radius: 3,
            timeoutMs: 180_000,
            log: (m) => this.log(`  ${m}`),
        });
        this.log(`boat fare v1 phase=done walkTo=${ok}`);
        ScriptRunner.stop(ok ? STOP_OK : STOP_FAIL);
    }
}
