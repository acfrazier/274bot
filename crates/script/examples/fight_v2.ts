import type { CombatOutcome, NativeApi } from '../host-js/index.d.ts';

/**
 * One native combat session per fight (JS API v2): kill `kills` cows in the
 * Lumbridge east cow field, then stop. The bot's own Combat machine owns the
 * fight (target pick, attacks, eating, prayers); the script only asks for
 * fights and reads their outcomes.
 *
 * Start the account inside the field (for example `::tele 0,50,51,55,14`,
 * tile 3255,3278) with a weapon and some food.
 */
export const apiVersion = 2;
export const SETTINGS = {
    kills: { type: 'number', default: 3, label: 'Cows to kill' },
};

/** The field's cow spawns, from the selected npc placements. */
const COW_FIELD = { min_x: 3240, min_z: 3254, max_x: 3266, max_z: 3299, level: 0 };

let kills = 0;

function describe(out: CombatOutcome): string {
    if (out.kind !== 'done') {
        return `${out.kind}: ${out.reason}`;
    }
    const end = out.value;
    switch (end.end) {
        case 'fought':
            return `fought: ${end.report.end}${end.report.reason ? ` (${end.report.reason})` : ''}`
                + ` in ${end.report.ticks} ticks, ${end.report.swings} swings, ate ${end.report.food}`;
        case 'interrupted':
            return `interrupted: ${end.cause}`;
        case 'refused':
        case 'failed':
            return `${end.end}: ${end.reason}`;
        case 'stopped':
            return 'stopped';
    }
}

// An async tick is not re-entered while its promise is pending, so each
// fight is awaited in order.
export async function tick(api: NativeApi): Promise<void> {
    if (!api.snapshot.ingame) {
        return;
    }
    const wanted = api.settings.num('kills', 3);
    const out = await api.combat.fight({
        target: { npc: 'Cow' },
        area: COW_FIELD,
        meleeMode: 'aggressive',
        budgetTicks: 500,
    });
    api.log(`fight ${kills + 1}: ${describe(out)}`);
    if (out.kind !== 'done' || out.value.end !== 'fought') {
        api.stop(`combat ${describe(out)}`);
        return;
    }
    if (out.value.report.end === 'killed') {
        kills += 1;
    } else if (out.value.report.end !== 'no-target' && out.value.report.end !== 'target-gone') {
        api.stop(`fight ended ${out.value.report.end}`);
        return;
    }
    if (kills >= wanted) {
        api.stop(`killed ${kills} cows`);
    }
}
