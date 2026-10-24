import type { CombatOutcome, CombatRequest, NativeApi } from '../host-js/index.d.ts';

/**
 * Native combat showcase (JS API v2): what the bot's own Combat machine does
 * when an external script drives it through `api.combat`. Every phase is one
 * `api.combat.fight` against the real cows of the Lumbridge east cow field;
 * the script only asks for fights, reads their reports and logs them.
 *
 * Per-account setup before Start (account cheats only; nothing is spawned).
 * The account must be past Tutorial Island in the session you start in
 * (`::getvar tutorial` says 1000; a brand-new account can be reset to 1 when
 * its character design closes, so set `::setvar tutorial 1000` after a relog):
 * the game only switches the combat tab to a wielded weapon past the tutorial,
 * and ranged and melee attack styles are set on that tab.
 *
 *   ::setstat attack 40      ::setstat strength 40    ::setstat defence 40
 *   ::setstat hitpoints 40   ::setstat ranged 30      ::setstat magic 30
 *   ::setstat prayer 43      (Protect from Melee needs 43)
 *   ::give bronze_scimitar 1 ::give shortbow 1        ::give bronze_arrow 300
 *   ::give airrune 300       ::give mindrune 300      ::give shrimp 20
 *   ::give 3doseprayerrestore 2
 *   ::setvar prayer0 1       (the user's own Thick Skin; it must stay on throughout)
 *   ::~stat_drain hitpoints 36 0   (4/40 HP, so the first fight eats)
 *   ::~stat_drain prayer 17 0      (26/43 points, at the prayer-potion line)
 *   ::tele 0,50,51,55,14     (tile 3255,3278, inside the cow field)
 *
 * Combat picks the weapon for each style from the inventory (scimitar, then
 * shortbow and arrows; magic casts Wind Strike with the carried runes).
 *
 * Phases, each logged as `[showcase] <phase> ...`:
 *  1. eat       — melee at low HP, prayers off: the bot eats when the cow hits back
 *  2. prayer    — the bot raises Protect from Melee, sips a prayer potion at the
 *                 potion line, and clears only its own prayer at the end; Thick Skin
 *                 (the user's) stays on
 *  3. aggressive / 4. defensive — melee, switching the attack style between fights
 *  5. ranged    — shortbow and bronze arrows, rapid
 *  6. magic     — one spell, Wind Strike, cast manually
 *  7. stop      — a fight stopped mid-swing with `api.combat.stop()`; the host
 *                 clears the bot's raised prayer and Thick Skin stays on
 */
export const apiVersion = 2;

const COW_FIELD = { min_x: 3240, min_z: 3254, max_x: 3266, max_z: 3299, level: 0 };
/** The whole field (earlier phases kill the nearest cows, so search wide);
 * each fight gets four minutes before it settles `budget`. */
const FIELD = { target: { npc: 'Cow' }, area: COW_FIELD, radius: 30, budgetTicks: 400 };
const USER_PRAYER = 'Thick Skin';
const BOT_PRAYER = 'Protect from Melee';
/** Ticks the stop phase lets the fight run once a cow is engaged. */
const STOP_AFTER_TICKS = 5;
/** Ticks between phases, so the next log shows the host's settled prayer state. */
const SETTLE_TICKS = 4;
/** A phase that found no cow (all killed, respawn pending) is asked again. */
const NO_TARGET_RETRIES = 3;
const RESPAWN_WAIT_TICKS = 10;

interface Phase {
    name: string;
    say: string;
    request: CombatRequest;
    stop?: boolean;
}

const PHASES: Phase[] = [
    {
        name: 'eat',
        say: 'melee at low HP with prayers off: watch the bot eat a shrimp',
        request: { ...FIELD, prayer: false, potions: false, food: true },
    },
    {
        name: 'prayer',
        say: 'the bot raises Protect from Melee and sips a prayer potion; Thick Skin stays on',
        request: { ...FIELD, prayer: true, potions: true },
    },
    {
        name: 'aggressive',
        say: 'melee, aggressive attack style (Strength XP)',
        request: { ...FIELD, style: 'melee', meleeMode: 'aggressive', prayer: false },
    },
    {
        name: 'defensive',
        say: 'melee, switched to the defensive attack style (Defence XP)',
        request: { ...FIELD, style: 'melee', meleeMode: 'defensive', prayer: false },
    },
    {
        name: 'ranged',
        say: 'ranged: Combat wields the carried shortbow and bronze arrows, rapid',
        request: { ...FIELD, style: 'ranged', rangedMode: 'rapid', prayer: false },
    },
    {
        name: 'magic',
        say: 'magic: one spell, Wind Strike, cast manually',
        request: { ...FIELD, style: 'magic', spells: ['wind_strike'], prayer: false },
    },
    {
        name: 'stop',
        say: `a fight stopped with api.combat.stop() ${STOP_AFTER_TICKS} ticks after it engages`,
        request: { ...FIELD, prayer: true },
        stop: true,
    },
];

let index = 0;
let live: Promise<CombatOutcome> | null = null;
let settled: CombatOutcome | null = null;
let engagedTicks = 0;
let stopSent = false;
let waitTicks = 0;
let retries = 0;

function log(api: NativeApi, message: string): void {
    api.log(`[showcase] ${message}`);
}

function prayerState(api: NativeApi): string {
    const user = api.prayerActive({ name: USER_PRAYER });
    const bot = api.prayerActive({ name: BOT_PRAYER });
    const points = api.prayerPoints();
    const show = (result: { ok: true; value: boolean } | { ok: false; error: string }) =>
        (result.ok ? (result.value ? 'on' : 'off') : result.error);
    return `${USER_PRAYER} ${show(user)}, ${BOT_PRAYER} ${show(bot)}, points ${points.ok ? points.value : points.error}`;
}

function hitpoints(api: NativeApi): string {
    const row = api.snapshot.stats.find((stat) => stat.index === 3);
    return row ? `${row.effective}/${row.base}` : '?';
}

function summary(out: CombatOutcome): string {
    if (out.kind !== 'done') {
        return `${out.kind}: ${out.reason}`;
    }
    const end = out.value;
    if (end.end === 'fought') {
        const r = end.report;
        return `fought → ${r.end}${r.reason ? ` (${r.reason})` : ''}: ${r.ticks} ticks, `
            + `${r.swings} swings, ${r.casts} casts, ${r.damageTaken} damage taken, ate ${r.food}, `
            + `prayer doses ${r.prayerDoses}, protect switches ${r.protectSwitches}`;
    }
    if (end.end === 'interrupted') {
        return `interrupted (${end.cause})`;
    }
    if (end.end === 'stopped') {
        return 'stopped';
    }
    return `${end.end}: ${end.reason}`;
}

export function tick(api: NativeApi): void {
    if (!api.snapshot.ingame) {
        return;
    }
    if (waitTicks > 0) {
        waitTicks -= 1;
        return;
    }
    if (index >= PHASES.length) {
        log(api, `done; ${prayerState(api)}`);
        api.stop('combat showcase complete');
        return;
    }
    const phase = PHASES[index];
    if (!live) {
        log(api, `${index + 1}/${PHASES.length} ${phase.name}: ${phase.say}`);
        log(api, `${phase.name} start: HP ${hitpoints(api)}; ${prayerState(api)}`);
        settled = null;
        engagedTicks = 0;
        stopSent = false;
        live = api.combat.fight(phase.request);
        live.then((out) => { settled = out; });
        return;
    }
    if (settled) {
        const out: CombatOutcome = settled;
        log(api, `${phase.name} end: ${summary(out)}; HP ${hitpoints(api)}; ${prayerState(api)}`);
        if (out.kind !== 'done' || out.value.end === 'refused' || out.value.end === 'failed') {
            api.stop(`showcase ${phase.name}: ${summary(out)}`);
            return;
        }
        live = null;
        if (out.value.end === 'fought' && out.value.report.end === 'no-target'
            && retries < NO_TARGET_RETRIES) {
            retries += 1;
            log(api, `${phase.name}: no cow in reach yet; asking again (${retries}/${NO_TARGET_RETRIES})`);
            waitTicks = RESPAWN_WAIT_TICKS;
            return;
        }
        retries = 0;
        index += 1;
        waitTicks = SETTLE_TICKS;
        return;
    }
    const session = api.snapshot.combat;
    if (session && session.status && session.status.stage === 'fighting'
        && session.status.engaged_kind === 'npc') {
        engagedTicks += 1;
        if (phase.stop && !stopSent && engagedTicks >= STOP_AFTER_TICKS) {
            const stopped = api.combat.stop();
            stopSent = true;
            log(api, `stop: api.combat.stop() → ${stopped.ok ? 'ok' : stopped.error}; ${prayerState(api)}`);
        }
    }
}
