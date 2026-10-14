import Tile from '../../../../../geometry/Tile.js';

export const MURDER_NAME = 'Murder Mystery';

export const MURDER_OBJ = {
    POT: 1931,
    POT_FLOUR: 1933,
    FLYPAPER: 1811,
    DAGGER: 1813,
    DAGGER_DUST: 1814,
    UNKNOWN_PRINT: 1822,
    KILLERS_PRINT: 1815,
    THREAD_RED: 1808,
    THREAD_GREEN: 1809,
    THREAD_BLUE: 1810,
};

export const MURDER_LOC = {
    WINDOW: 2666,
    FLOUR_BARREL: 2662,
    SACKS: 2663,
};

export const MURDER_TILE = {
    BANK: new Tile(2725, 3491, 0),
    GUARD: new Tile(2741, 3562, 0),
    STUDY: new Tile(2747, 3578, 0),
    FLOUR_BARREL: new Tile(2735, 3581, 0),
    SACKS: new Tile(2732, 3581, 0),
    SALESMAN: new Tile(2737, 3491, 0),
    ARHEIN: new Tile(2803, 3430, 0),
};

export const ARHEIN = { npc: 'Arhein', anchor: MURDER_TILE.ARHEIN };

export const GUARD_START = {
    npc: 'Guard',
    anchor: MURDER_TILE.GUARD,
    leash: 8,
    prefer: ["Sure, I'll help"],
};

export const GUARD_HANDIN = {
    npc: 'Guard',
    anchor: MURDER_TILE.GUARD,
    leash: 8,
    prefer: ['I know who did it'],
};

export const SALESMAN = {
    npc: 'Poison Salesman',
    anchor: MURDER_TILE.SALESMAN,
    leash: 10,
    prefer: ['Talk about the Murder Mystery Quest', 'Who did you sell Poison to at the house'],
};

const POISON_ASK = ['buy poison the other day'];

export const SUSPECTS = [
    {
        stop: { npc: 'Anna', anchor: new Tile(2734, 3575, 0), leash: 8, prefer: POISON_ASK },
        barrel: { id: 2656, name: "Annas' barrel", near: new Tile(2734, 3575, 0) },
        silver: 1796,
        dust: 1797,
        print: 1816,
        thread: MURDER_OBJ.THREAD_GREEN,
        poison: { id: 2650, name: 'Sinclair family compost heap', near: new Tile(2732, 3572, 0) },
    },
    {
        stop: { npc: 'Bob', anchor: new Tile(2748, 3559, 0), leash: 8, prefer: POISON_ASK },
        barrel: { id: 2657, name: "Bobs' barrel", near: new Tile(2734, 3578, 0) },
        silver: 1798,
        dust: 1799,
        print: 1817,
        thread: MURDER_OBJ.THREAD_RED,
        poison: { id: 2651, name: 'Sinclair family beehive', near: new Tile(2731, 3559, 0) },
    },
    {
        stop: { npc: 'Carol', anchor: new Tile(2734, 3581, 1), leash: 8, prefer: POISON_ASK },
        barrel: { id: 2658, name: "Carols' barrel", near: new Tile(2734, 3581, 1) },
        silver: 1800,
        dust: 1801,
        print: 1818,
        thread: MURDER_OBJ.THREAD_RED,
        poison: { id: 2652, name: 'Sinclair mansion drain', near: new Tile(2736, 3572, 0) },
    },
    {
        stop: { npc: 'David', anchor: new Tile(2739, 3581, 0), leash: 8, prefer: POISON_ASK },
        barrel: { id: 2659, name: "Davids' barrel", near: new Tile(2734, 3578, 1) },
        silver: 1802,
        dust: 1803,
        print: 1819,
        thread: MURDER_OBJ.THREAD_GREEN,
        poison: { id: 2653, name: "Spiders' nest", near: new Tile(2740, 3575, 1) },
    },
    {
        stop: { npc: 'Elizabeth', anchor: new Tile(2746, 3581, 1), leash: 8, prefer: POISON_ASK },
        barrel: { id: 2660, name: "Elizabeths' barrel", near: new Tile(2746, 3581, 1) },
        silver: 1804,
        dust: 1805,
        print: 1820,
        thread: MURDER_OBJ.THREAD_BLUE,
        poison: { id: 2654, name: 'Sinclair family fountain', near: new Tile(2746, 3563, 0) },
    },
    {
        stop: { npc: 'Frank', anchor: new Tile(2742, 3577, 0), leash: 8, prefer: POISON_ASK },
        barrel: { id: 2661, name: "Franks' barrel", near: new Tile(2746, 3577, 1) },
        silver: 1806,
        dust: 1807,
        print: 1821,
        thread: MURDER_OBJ.THREAD_BLUE,
        poison: { id: 2655, name: 'Sinclair family crest', near: new Tile(2745, 3573, 0) },
    },
];

export function suspectOrder(thread) {
    return globalThis.__rs2b0t_murder_areas('suspectOrder', thread, SUSPECTS);
}

export const MURDER_EVIDENCE = [
    { id: MURDER_OBJ.DAGGER, name: "Criminals' dagger" },
    { id: MURDER_OBJ.DAGGER_DUST, name: "Criminals' dagger" },
    { id: MURDER_OBJ.UNKNOWN_PRINT, name: 'Unknown print' },
    { id: MURDER_OBJ.KILLERS_PRINT, name: "Killers' print" },
    { id: MURDER_OBJ.THREAD_RED, name: "Criminals' thread" },
    { id: MURDER_OBJ.THREAD_GREEN, name: "Criminals' thread" },
    { id: MURDER_OBJ.THREAD_BLUE, name: "Criminals' thread" },
    { id: 1796, name: 'silver keepsake' },
    { id: 1797, name: 'floured keepsake' },
    { id: 1816, name: 'suspect print' },
    { id: 1798, name: 'silver keepsake' },
    { id: 1799, name: 'floured keepsake' },
    { id: 1817, name: 'suspect print' },
    { id: 1800, name: 'silver keepsake' },
    { id: 1801, name: 'floured keepsake' },
    { id: 1818, name: 'suspect print' },
    { id: 1802, name: 'silver keepsake' },
    { id: 1803, name: 'floured keepsake' },
    { id: 1819, name: 'suspect print' },
    { id: 1804, name: 'silver keepsake' },
    { id: 1805, name: 'floured keepsake' },
    { id: 1820, name: 'suspect print' },
    { id: 1806, name: 'silver keepsake' },
    { id: 1807, name: 'floured keepsake' },
    { id: 1821, name: 'suspect print' },
];
