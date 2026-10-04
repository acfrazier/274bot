import fs from 'node:fs';
import path from 'node:path';
import { parsePack, parseParamDefinitions, requireGatherText, walkContentFiles } from './common.ts';
import { buildSpellMaxHits, extractNpcCombatFacts, parseCombatScripts, type CombatNpcSource, type CombatScripts } from './combat.ts';

export const npcNamesContentFiles = ['pack/npc.pack', 'scripts/skill_combat/configs/combat.param'];

type NpcBlock = {
    name: string;
    display: string | null;
    ops: string[];
    size: number;
    wanderrange: number;
    maxrange: number;
    attackrange: number;
    huntrange: number;
    vislevel: number;
    headicon: number | null;
    hitpoints: number;
    damagetype: string | null;
    category: string | null;
    strength: number | null;
    ranged: number | null;
    strengthbonus: number | null;
    rangebonus: number | null;
    undead: number | null;
    attackrate: number | null;
    params: Map<string, string>;
};

function parseIntField(raw: string | undefined, _key: string, fallback: number) {
    if (raw === undefined || raw === '') return fallback;
    const parsed = Number(raw);
    if (!Number.isInteger(parsed)) return fallback;
    return parsed;
}

function parseCombatInt(raw: string | undefined, label: string): number | null {
    if (raw === undefined || raw === '') return null;
    const parsed = Number(raw);
    if (!Number.isSafeInteger(parsed)) throw new Error(`npc_names: ${label} is not an integer: ${raw}`);
    return parsed;
}

function parseParamInt(raw: string | undefined, label: string): number | null {
    if (raw === '^true' || raw === 'true') return 1;
    if (raw === '^false' || raw === 'false') return 0;
    return parseCombatInt(raw, label);
}

function parseNpcFiles(content: string, revision: 274 | 289) {
    const definitions = parseParamDefinitions(requireGatherText(content, 'scripts/skill_combat/configs/combat.param'));
    const defaultMaxrange = revision === 274 ? 7 : -1;
    const defaultParam = (name: string) => {
        const definition = definitions.get(name);
        if (definition?.type !== undefined && definition.type !== 'int') {
            throw new Error(`npc_names: ${name} is not an integer parameter`);
        }
        const value = parseParamInt(definition?.default, `${name} default`);
        if (name === 'attackrate' && value !== null && (value < 0 || value > 255)) {
            throw new Error(`npc_names: ${name} default is out of range: ${value}`);
        }
        return value;
    };
    const strengthbonus = defaultParam('strengthbonus');
    const rangebonus = defaultParam('rangebonus');
    const attackrate = defaultParam('attackrate');
    const blocks = new Map<string, NpcBlock>();
    const files: string[] = [];
    for (const file of walkContentFiles(path.join(content, 'scripts'), '.npc')) {
        files.push(path.relative(content, file).split(path.sep).join('/'));
        let current: NpcBlock | null = null;
        for (const raw of fs.readFileSync(file, 'utf8').split(/\r?\n/)) {
            const line = raw.trim();
            if (!line || line.startsWith('//')) continue;
            if (line.startsWith('[') && line.endsWith(']')) {
                const name = line.slice(1, -1);
                if (blocks.has(name)) throw new Error(`npc_names: duplicate config [${name}]`);
                current = {
                    name,
                    display: null,
                    ops: [],
                    size: 1,
                    wanderrange: 5,
                    maxrange: defaultMaxrange,
                    attackrange: 0,
                    huntrange: 0,
                    vislevel: 0,
                    headicon: null,
                    hitpoints: 0,
                    damagetype: null,
                    category: null,
                    strength: null,
                    ranged: null,
                    strengthbonus,
                    rangebonus,
                    undead: null,
                    attackrate,
                    params: new Map(),
                };
                blocks.set(name, current);
                continue;
            }
            if (!current) continue;
            const eq = line.indexOf('=');
            if (eq <= 0) continue;
            const key = line.slice(0, eq);
            const value = line.slice(eq + 1);
            if (key === 'name') current.display = value;
            else if (/^op[1-5]$/.test(key) && value) current.ops.push(value);
            else if (key === 'size') current.size = parseIntField(value, key, 1);
            else if (key === 'wanderrange') current.wanderrange = parseIntField(value, key, 5);
            else if (key === 'maxrange') current.maxrange = parseIntField(value, key, defaultMaxrange);
            else if (key === 'attackrange') current.attackrange = parseIntField(value, key, 0);
            else if (key === 'huntrange') current.huntrange = parseIntField(value, key, 0);
            else if (key === 'vislevel') current.vislevel = value === 'hide' ? 0 : parseIntField(value, key, 0);
            else if (key === 'headicon') current.headicon = parseCombatInt(value, `${current.name}.headicon`);
            else if (key === 'hitpoints') current.hitpoints = parseIntField(value, key, 0);
            else if (key === 'category') current.category = value;
            else if (key === 'strength') current.strength = parseCombatInt(value, `${current.name}.strength`);
            else if (key === 'ranged') current.ranged = parseCombatInt(value, `${current.name}.ranged`);
            else if (key === 'param') {
                const comma = value.indexOf(',');
                if (comma <= 0) continue;
                const param = value.slice(0, comma);
                const rawValue = value.slice(comma + 1);
                current.params.set(param, rawValue);
                if (param === 'damagetype') current.damagetype = rawValue;
                else if (param === 'strengthbonus') current.strengthbonus = parseParamInt(rawValue, `${current.name}.strengthbonus`);
                else if (param === 'rangebonus') current.rangebonus = parseParamInt(rawValue, `${current.name}.rangebonus`);
                else if (param === 'undead') current.undead = parseParamInt(rawValue, `${current.name}.undead`);
                else if (param === 'attackrate') {
                    const rate = parseCombatInt(rawValue, `${current.name}.attackrate`);
                    if (rate !== null && (rate < 0 || rate > 255)) throw new Error(`npc_names: ${current.name}.attackrate is out of range: ${rawValue}`);
                    current.attackrate = rate;
                }
            }
        }
    }
    return { blocks, files };
}

export function extractNpcNamesFacts(content: string, revision: number, scripts: CombatScripts = parseCombatScripts(content), maxHits = buildSpellMaxHits(content)) {
    if (revision !== 274 && revision !== 289) throw new Error(`npc_names: unsupported revision ${revision}`);
    const pack = parsePack(requireGatherText(content, 'pack/npc.pack'));
    if (pack.size === 0) throw new Error('npc_names: empty pack/npc.pack');
    const { blocks, files } = parseNpcFiles(content, revision);
    const combatNpcs: CombatNpcSource[] = [];
    const rows = [...pack.entries()]
        .sort((a, b) => a[1] - b[1])
        .map(([config, id]) => {
            const block = blocks.get(config);
            // Pinned NpcType.ts: 274 defaults at 102-103; 289 defaults/postDecode at 101-102,118-125.
            const wanderrange = block?.wanderrange ?? 5;
            let maxrange = block?.maxrange ?? (revision === 274 ? 7 : -1);
            if (revision === 289) {
                if (maxrange === -1) maxrange = wanderrange + 2;
                maxrange = Math.max(maxrange, wanderrange);
            }
            const combat = extractNpcCombatFacts(config, block?.category ?? null, scripts, maxHits);
            combatNpcs.push({
                config,
                category: block?.category ?? null,
                params: block?.params ?? new Map(),
                attack_kind: combat.attack_kind,
                dragonfire: combat.dragonfire,
            });
            return {
                id,
                config,
                display: block?.display ?? null,
                ops: block?.ops ?? [],
                size: block?.size ?? 1,
                wanderrange,
                maxrange,
                attackrange: block?.attackrange ?? 0,
                huntrange: block?.huntrange ?? 0,
                vislevel: block?.vislevel ?? 0,
                headicon: block?.headicon ?? undefined,
                hitpoints: block?.hitpoints ?? 0,
                damagetype: block?.damagetype ?? null,
                strength: block?.strength ?? null,
                ranged: block?.ranged ?? null,
                strengthbonus: block?.strengthbonus ?? null,
                rangebonus: block?.rangebonus ?? null,
                undead: block?.undead ?? null,
                ap_attack: combat.ap_attack,
                attack_kind: combat.attack_kind,
                forced_max_hit: combat.forced_max_hit,
                dragonfire: combat.dragonfire,
                attackrate: block?.attackrate ?? null,
                bespoke: combat.bespoke,
                counter_protect: combat.counter_protect,
            };
        });
    if (rows.length === 0) throw new Error('npc_names: no extracted rows');
    if (!rows.some((row) => row.config === 'khazard_warlord' && row.id === 477)) {
        throw new Error('npc_names: missing khazard_warlord pack join');
    }
    return { rows, files, combatNpcs };
}
