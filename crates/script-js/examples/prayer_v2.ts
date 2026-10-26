type NativeApi = import('../host-js/index.d.ts').NativeApi;

/**
 * Headed File-card v2 witness. Sequential query → unknown refuse →
 * Protect from Melee ON → clear with timed_out 0 → named stop.
 * Reads NativeSnapshot `ingame` / `here` only (no `scene_state`).
 */
export const apiVersion = 2;

const STOP_OK = 'prayer v2 qualification complete';
const MELEE = 'Protect from Melee';

export async function tick(api: NativeApi) {
    const snap = api.snapshot;
    if (!snap.ingame || !snap.here) {
        return;
    }
    const known = api.prayerKnown({ name: MELEE });
    const avail = api.prayerAvailable({ name: MELEE });
    const active = api.prayerActive({ name: MELEE });
    const pts = api.prayerPoints();
    const max = api.prayerMax();
    const full = api.prayerFull();
    if (!known.ok || !avail.ok || !active.ok || !pts.ok || !max.ok || !full.ok) {
        throw known.error || avail.error || active.error || pts.error || max.error || full.error;
    }
    api.log(
        `prayer v2 query points=${pts.value} max=${max.value} known=${known.value} avail=${avail.value} active=${active.value}`,
    );
    api.paint
        .begin()
        .title('prayer v2')
        .row('phase', 'query')
        .row(pts.value, '/', max.value, full.value ? 'full' : '')
        .row(known.value ? 'known' : 'unknown', avail.value ? 'avail' : '', active.value ? 'on' : 'off')
        .end();

    const unknown = await api.prayerSet({ name: 'Nope', on: true });
    if (unknown.ok) {
        throw 'unknown prayerSet must not succeed';
    }
    api.log(`prayer v2 unknown-set error=${unknown.error}`);

    api.paint.begin().title('prayer v2').row('phase', 'set melee').end();
    const set = await api.prayerSet({ name: MELEE, on: true });
    if (!set.ok) {
        throw set.error;
    }
    api.log('prayer v2 set Protect from Melee ok');

    api.paint.begin().title('prayer v2').row('phase', 'clear').end();
    const cleared = await api.prayerClear();
    if (!cleared.ok) {
        throw cleared.error;
    }
    if (cleared.value.timed_out !== 0) {
        throw `prayerClear timed_out=${cleared.value.timed_out}; walk ok is not LIVE PASS`;
    }
    api.log(`prayer v2 clear clicked=${cleared.value.clicked} timed_out=0`);
    api.paint.begin().title('prayer v2').row('phase', 'done').row('result', STOP_OK).end();
    api.stop(STOP_OK);
}
