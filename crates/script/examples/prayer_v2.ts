type NativeApi = import('../host-js/index.d.ts').NativeApi;

/**
 * External v2 prayer helper: query, optional set, then clear and stop.
 * Does not loop or toggle forever. prayerClear ok means the 15-row walk
 * finished; timed_out may be nonzero. LIVE later requires all varps off.
 */
export const apiVersion = 2;

export async function tick(api: NativeApi) {
    const name = 'Protect from Melee';
    const known = api.prayerKnown({ name });
    if (!known.ok) throw known.error;
    const avail = api.prayerAvailable({ name });
    if (!avail.ok) throw avail.error;
    const active = api.prayerActive({ name });
    if (!active.ok) throw active.error;
    const pts = api.prayerPoints();
    const max = api.prayerMax();
    const full = api.prayerFull();
    if (!pts.ok || !max.ok || !full.ok) {
        throw pts.error || max.error || full.error;
    }
    api.paint
        .begin()
        .title('prayer v2')
        .row(pts.value, '/', max.value, full.value ? 'full' : '')
        .row(known.value ? 'known' : 'unknown', avail.value ? 'avail' : '', active.value ? 'on' : 'off')
        .end();
    if (known.value && avail.value && !active.value) {
        const set = await api.prayerSet({ name, on: true });
        if (!set.ok) throw set.error;
    }
    const cleared = await api.prayerClear();
    if (!cleared.ok) throw cleared.error;
    api.log(
        `prayer v2 clear clicked=${cleared.value.clicked} timed_out=${cleared.value.timed_out}`,
    );
    api.stop();
}
