export const ROCK_OPTIONS = [
    'Clay',
    'Copper',
    'Tin',
    'Iron',
    'Silver',
    'Coal',
    'Gold',
    'Mithril',
    'Adamantite',
    'Runite',
];

export const ROCK_TYPES = Object.fromEntries(ROCK_OPTIONS.map((n) => [n, []]));
export const QUEST_ROCK_TYPES = {};
export const GAS_ROCK_IDS = new Set();
export const GAS_ROCK_TICKS = 60;
export const BROKEN_PICKAXE = 'Broken pickaxe';

export function resolveRockIds() {
    return new Set();
}
