type NativeApi = import('../host-js/index.d.ts').NativeApi;

/** Read-only supply helper smoke example (offline isolate; not gameplay acceptance). */
export const apiVersion = 2;

export function tick(api: NativeApi): void {
    const sharkSlots = api.foodCount({
        items: api.snapshot.inv,
        foodName: 'Shark',
    });
    const breadHeal = api.foodHealAmount({ foodName: 'Bread' });
    const keep = api.combatKeepNames({
        food: 'Lobster',
        style: 'mage',
        spell: 'Wind Strike',
    });
    const runes = api.runesPerCast({
        spellName: 'Wind Strike',
        wielded: ['Staff of air'],
    });
    const escape = api.escapeRunesFor({ id: 'varrock' });
    api.log(
        JSON.stringify({
            sharkSlots,
            breadHeal,
            keep,
            runes,
            escape,
        }),
    );
}
