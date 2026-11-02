import { parsePack, requireGatherText } from './common.ts';

// PvP V0 duel facts (PVP-DESIGN-1026 §3.2, §3.3, §7.6).
//
// Rule buttons: the 14 `duel_select_type:<rule>` toggles of
// `minigames/game_duelarena/scripts/duel_arena_settings.rs2:1-85`, indexed by
// the bit values of `minigames/game_duelarena/configs/duelarena.constant:1-14`.
// Zone constants: the six `coord_pair` rows of
// `minigames/game_duelarena/configs/duelarena.dbrow:1-11`, the challenge row
// `:13-15`, and the lobby telejump of
// `minigames/game_duelarena/scripts/duel_arena_finish.rs2:75`.

export type DuelRuleButtonWire = { bit: number; component: number };

export type DuelRectWire = { x1: number; z1: number; x2: number; z2: number };

export type DuelZoneFactsWire = {
    pens: ({ x1: number; z1: number; x2: number; z2: number; obstacles: boolean })[];
    challenge: DuelRectWire;
    lobby: DuelRectWire;
};

// Button names as written in `duel_arena_settings.rs2`, in bit order per
// `duelarena.constant:1-14` (`NoForfeit = 0` through `NoSpecs = 13`).
const RULE_BUTTONS = [
    'no_forfeit',
    'no_movement',
    'no_weapons',
    'no_armour',
    'no_ranged',
    'no_melee',
    'no_magic',
    'no_potions',
    'no_food',
    'no_prayer',
    'obstacles',
    'no_jewelry',
    'flower_power',
    'no_specs',
] as const;

export function extractDuelRuleButtons(content: string): DuelRuleButtonWire[] {
    const interfaces = parsePack(requireGatherText(content, 'pack/interface.pack'));
    const bits = new Map<string, number>();
    for (const raw of requireGatherText(content, 'scripts/minigames/game_duelarena/configs/duelarena.constant').split(/\r?\n/)) {
        const line = raw.trim();
        if (!line.startsWith('^')) continue;
        const match = /^\^([A-Za-z][A-Za-z0-9_]*)\s*=\s*(\d+)\s*$/.exec(line);
        if (!match) continue;
        bits.set(match[1]!, Number(match[2]));
    }
    // The settings script owns the button set: every `if_button` section is
    // one rule button, and the set must be exactly the 14 known rules.
    const settings = requireGatherText(content, 'scripts/minigames/game_duelarena/scripts/duel_arena_settings.rs2');
    const buttons = [...settings.matchAll(/^\[if_button,duel_select_type:([A-Za-z][A-Za-z0-9_]*)\]/gm)].map((match) => match[1]!);
    const names = [...new Set(buttons)].sort();
    const expected = [...RULE_BUTTONS].sort();
    if (JSON.stringify(names) !== JSON.stringify(expected)) {
        throw new Error(`duel rules: settings buttons [${names.join(',')}] are not the 14 known rules`);
    }
    return RULE_BUTTONS.map((rule) => {
        const bit = bits.get(rule);
        if (bit === undefined) throw new Error(`duel rules: duelarena.constant lacks ^${rule}`);
        const component = interfaces.get(`duel_select_type:${rule}`);
        if (component === undefined) throw new Error(`duel rules: interface.pack lacks duel_select_type:${rule}`);
        return { bit, component };
    }).sort((a, b) => a.bit - b.bit);
}

function decodeCorner(value: string, label: string): { level: number; x: number; z: number } {
    const parts = value.split('_').map(Number);
    if (parts.length !== 5 || parts.some((part) => !Number.isInteger(part) || part < 0)) {
        throw new Error(`${label}: bad coord corner ${value}`);
    }
    const [level, mx, mz, lx, lz] = parts as number[];
    return { level: level!, x: mx! * 64 + lx!, z: mz! * 64 + lz! };
}

export function extractDuelZones(content: string): DuelZoneFactsWire {
    const dbrow = requireGatherText(content, 'scripts/minigames/game_duelarena/configs/duelarena.dbrow');
    const tables = new Map<string, string[][]>();
    let current: string | null = null;
    for (const raw of dbrow.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            current = line.slice(1, -1);
            continue;
        }
        if (current && line.startsWith('data=')) {
            const [, rest] = line.split('=', 2);
            const [key, ...values] = rest!.split(',');
            if (key === 'coord_pair') (tables.get(current) ?? tables.set(current, []).get(current)!).push(values);
        }
    }
    const coordPairs = (table: string): string[][] => {
        const rows = tables.get(table);
        if (!rows) throw new Error(`duel zones: duelarena.dbrow lacks ${table}`);
        return rows;
    };
    const rect = (values: string[], label: string): DuelRectWire => {
        if (values.length !== 2) throw new Error(`${label}: expected one coord_pair, got ${values.join(',')}`);
        const first = decodeCorner(values[0]!, label);
        const second = decodeCorner(values[1]!, label);
        if (first.level !== 0 || second.level !== 0) throw new Error(`${label}: duel areas are level 0, got ${values.join(',')}`);
        if (first.x > second.x || first.z > second.z) {
            throw new Error(`${label}: reversed coord_pair ${values.join(',')}`);
        }
        return { x1: first.x, z1: first.z, x2: second.x, z2: second.z };
    };
    const fights = coordPairs('duel_arena_fight_zones');
    const obstacles = coordPairs('duel_arena_obstacle_fight_zones');
    if (fights.length !== 3 || obstacles.length !== 3) {
        throw new Error(`duel zones: expected 3 fight and 3 obstacle pens, got ${fights.length}/${obstacles.length}`);
    }
    const pens = [
        ...fights.map((values) => ({ ...rect(values, 'duel_arena_fight_zones'), obstacles: false as const })),
        ...obstacles.map((values) => ({ ...rect(values, 'duel_arena_obstacle_fight_zones'), obstacles: true as const })),
    ];
    const challengeRows = coordPairs('duel_arena_zones');
    if (challengeRows.length !== 1) throw new Error(`duel zones: expected 1 challenge row, got ${challengeRows.length}`);
    const challenge = rect(challengeRows[0]!, 'duel_arena_zones');
    const finish = requireGatherText(content, 'scripts/minigames/game_duelarena/scripts/duel_arena_finish.rs2');
    const lobbyMatch = /map_findsquare_inzone\((0_[\d_]+),\s*(0_[\d_]+)\)/.exec(finish);
    if (!lobbyMatch) throw new Error('duel zones: duel_arena_finish.rs2 lacks the lobby telejump coords');
    const lobbyFirst = decodeCorner(lobbyMatch[1]!, 'duel lobby');
    const lobbySecond = decodeCorner(lobbyMatch[2]!, 'duel lobby');
    if (lobbyFirst.level !== 0 || lobbySecond.level !== 0) throw new Error('duel zones: duel lobby is level 0');
    const lobby: DuelRectWire = {
        x1: Math.min(lobbyFirst.x, lobbySecond.x), z1: Math.min(lobbyFirst.z, lobbySecond.z),
        x2: Math.max(lobbyFirst.x, lobbySecond.x), z2: Math.max(lobbyFirst.z, lobbySecond.z),
    };
    return { pens, challenge, lobby };
}
