type NativeApi = import('../host-js/index.d.ts').NativeApi;

/** Read-only loadout/potion helper smoke example (offline isolate; not gameplay). */
export const apiVersion = 2;

export function tick(api: NativeApi): void {
    const loadout = {
        name: 'melee',
        worn: {
            hat: 'Rune full helm',
            righthand: 'Rune scimitar',
            torso: 'Rune chainbody',
        },
        carry: [
            { item: 'Lobster', qty: 10 },
            { item: 'Super attack(4)', qty: 2 },
            { item: 'Prayer potion(4)', qty: 1 },
        ],
        unassigned: ['old helm'],
    };
    const food = api.foodOf({ loadout, fallback: 'Trout' });
    const gear = api.gearOf({ loadout });
    const supplies = api.suppliesOf({ loadout });
    const weapon = api.weaponOf({ loadout, fallback: 'Bronze sword' });
    const range = api.rangeLoadoutOf({ weapon: 'Bronze dart', ammo: 'Iron arrow' });
    const attack = api.snapshot.stats.find((row) => row.name.toLowerCase() === 'attack');
    const faded = api.boostFaded({
        base: attack ? attack.base : 1,
        effective: attack ? attack.effective : 1,
    });
    const planned = api.plannedPotions({
        carry: [
            { item: 'Super attack(4)', qty: 2 },
            { item: 'Lobster', qty: 10 },
        ],
    });
    const plans = planned.ok ? planned.value : [];
    const held = plans.map((plan) =>
        plan.doses.reduce(
            (n, dose) =>
                n +
                api.snapshot.inv
                    .filter((item) => item.name === dose)
                    .reduce((sum, item) => sum + item.count, 0),
            0,
        ),
    );
    const sip = api.potionToSip({
        plans,
        held,
        levels: api.snapshot.stats.map((row) => ({
            skill: row.name,
            base: row.base,
            effective: row.effective,
        })),
    });
    api.log(
        JSON.stringify({
            food,
            gear,
            supplies,
            weapon,
            range,
            faded,
            planned,
            sip,
        }),
    );
}
