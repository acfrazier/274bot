import { Prayer } from '../../api/prayer/Prayer.js';
import { ScriptRunner } from '../../runtime/ScriptRunner.js';

/**
 * Qualification-only File adapter. Imports the frozen Prayer shim path
 * and keeps v1 boolean/void results. Not a claimed catalog whale.
 */
const STOP_OK = 'prayer v1 qualification complete';
const MELEE = 'Protect from Melee';

export default class PrayerV1Qualification extends LoopingBot {
    async loop() {
        if (globalThis.__prayer_v1_done) {
            return;
        }
        const points = Prayer.points();
        const max = Prayer.max();
        const known = Prayer.known(MELEE);
        const avail = Prayer.available(MELEE);
        const active = Prayer.active(MELEE);
        this.log(
            `prayer v1 query points=${points} max=${max} known=${known} avail=${avail} active=${active}`,
        );
        this.log('prayer v1 phase=query');

        const unknown = await Prayer.set('Nope', true);
        if (unknown) {
            throw new Error('unknown Prayer.set must be false');
        }
        this.log('prayer v1 unknown-set false');

        this.log('prayer v1 phase=set melee');
        const set = await Prayer.set(MELEE, true);
        if (!set) {
            throw new Error('Prayer.set Protect from Melee returned false');
        }
        this.log('prayer v1 set Protect from Melee true');

        this.log('prayer v1 phase=clear');
        await Prayer.clear();
        if (Prayer.active(MELEE)) {
            throw new Error('Protect from Melee still active after clear');
        }
        this.log('prayer v1 clear void melee-off');
        this.log(`prayer v1 phase=done result=${STOP_OK}`);
        globalThis.__prayer_v1_done = true;
        ScriptRunner.stop(STOP_OK);
    }
}
