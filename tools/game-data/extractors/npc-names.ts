import fs from 'node:fs';
import path from 'node:path';
import { parsePack, parseParamDefinitions, requireGatherText, walkContentFiles } from './common.ts';
import { buildSpellMaxHits, combatConstantCodes, extractNpcCombatFacts, extractNpcCombatTransformTarget, parseCombatScripts, type CombatNpcSource, type CombatScripts, type NpcCombatTransformTarget } from './combat.ts';

export const NPC_COMBAT_FACTS_VERSION = 2;

export const npcNamesContentFiles = [
    'pack/npc.pack',
    'scripts/skill_combat/configs/combat.param',
    'scripts/skill_combat/configs/npc_combat.param',
    'scripts/skill_combat/configs/combat_damagetypes.constant',
];

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
    attack: number;
    defence: number;
    strength: number;
    hitpoints: number;
    ranged: number;
    magic: number;
    damagetype: string | null;
    category: string | null;
    attackbonus: number;
    rangeattack: number;
    magicattack: number;
    stabdefence: number;
    slashdefence: number;
    crushdefence: number;
    magicdefence: number;
    rangedefence: number;
    strengthbonus: number;
    rangebonus: number;
    max_dealt: number;
    undead: number | null;
    attackrate: number;
    poison_severity: number;
    forcemulti: boolean;
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
function requiredCombatInt(raw: string | undefined, label: string): number {
    if (raw === undefined || raw.trim() === '') throw new Error(`npc_names: ${label} is missing`);
    const parsed = Number(raw);
    if (!Number.isSafeInteger(parsed)) throw new Error(`npc_names: ${label} is not an integer: ${raw}`);
    return parsed;
}

function requiredParamInt(raw: string | undefined, label: string): number {
    if (raw === '^true' || raw === 'true') return 1;
    if (raw === '^false' || raw === 'false') return 0;
    return requiredCombatInt(raw, label);
}

const NPC_STAT_NAMES = ['attack', 'defence', 'strength', 'hitpoints', 'ranged', 'magic'] as const;
type NpcStatDefaults = Record<typeof NPC_STAT_NAMES[number], number>;

function parseNpcStatDefaults(engineNpcTypeSource: string, revision: number): NpcStatDefaults {
    const declarations = [...engineNpcTypeSource.matchAll(/^\s*stats\s*=\s*\[([^\]]*)\]\s*;/gm)];
    if (declarations.length !== 1) {
        throw new Error(`npc_names: ${revision} NpcType.ts is missing the engine default for hitpoints`);
    }
    const values = declarations[0]![1]!.split(',').map((value) => value.trim());
    const defaults = {} as NpcStatDefaults;
    for (let index = 0; index < NPC_STAT_NAMES.length; index++) {
        const name = NPC_STAT_NAMES[index]!;
        const raw = values[index];
        if (raw === undefined || raw === '') {
            throw new Error(`npc_names: ${revision} NpcType.ts is missing the engine default for ${name}`);
        }
        defaults[name] = requiredCombatInt(raw, `${revision} NpcType.ts ${name} default`);
    }
    if (values.length !== NPC_STAT_NAMES.length) {
        throw new Error(`npc_names: ${revision} NpcType.ts has ${values.length} stat defaults; expected ${NPC_STAT_NAMES.length}`);
    }
    return defaults;
}

function requiredParamDefault(definitions: ReadonlyMap<string, { type?: string; default?: string }>, name: string, file = 'combat.param'): number {
    const definition = definitions.get(name);
    if (definition?.type?.toLowerCase() !== 'int') {
        throw new Error(`npc_names: ${name} is missing an integer ${file} default`);
    }
    return requiredParamInt(definition.default, `${name} ${file} default`);
}

/** Parse the selected engine's six positional NPC stat defaults. */
export function parseNpcFiles(content: string, revision: 274 | 289 | 377, engineNpcTypeSource: string) {
    const definitions = parseParamDefinitions(requireGatherText(content, 'scripts/skill_combat/configs/combat.param'));
    const stats = parseNpcStatDefaults(engineNpcTypeSource, revision);
    const npcDefinitions = parseParamDefinitions(requireGatherText(content, 'scripts/skill_combat/configs/npc_combat.param'));
    const damageCode = requiredParamDefault(npcDefinitions, 'damagetype', 'npc_combat.param');
    const damageCodes = combatConstantCodes(content, 'scripts/skill_combat/configs/combat_damagetypes.constant', 'damagetype');
    const damageNames = [...damageCodes].filter(([, code]) => code === damageCode);
    if (damageNames.length !== 1) throw new Error(`npc_names: damagetype default ${damageCode} has no unique content constant`);
    const damagetype = `^${damageNames[0]![0]}`;
    const defaultMaxrange = revision === 274 ? 7 : -1;
    const attackbonus = requiredParamDefault(definitions, 'attackbonus');
    const rangeattack = requiredParamDefault(definitions, 'rangeattack');
    const magicattack = requiredParamDefault(definitions, 'magicattack');
    const stabdefence = requiredParamDefault(definitions, 'stabdefence');
    const slashdefence = requiredParamDefault(definitions, 'slashdefence');
    const crushdefence = requiredParamDefault(definitions, 'crushdefence');
    const magicdefence = requiredParamDefault(definitions, 'magicdefence');
    const rangedefence = requiredParamDefault(definitions, 'rangedefence');
    const strengthbonus = requiredParamDefault(definitions, 'strengthbonus');
    const rangebonus = requiredParamDefault(definitions, 'rangebonus');
    const attackrate = requiredParamDefault(definitions, 'attackrate');
    if (attackrate < 0 || attackrate > 255) {
        throw new Error(`npc_names: attackrate default is out of range: ${attackrate}`);
    }
    const poison_severity = requiredParamDefault(definitions, 'poison_severity');
    const forcemulti = requiredParamDefault(definitions, 'npc_forcemulti') !== 0;
    const max_dealt = requiredParamDefault(definitions, 'max_dealt');
    const combatDefaults = {
        damagetype,
        attackbonus,
        rangeattack,
        magicattack,
        stabdefence,
        slashdefence,
        crushdefence,
        magicdefence,
        rangedefence,
        strengthbonus,
        rangebonus,
        attackrate,
        poison_severity,
        forcemulti,
        max_dealt,
    };
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
                    attack: stats.attack,
                    defence: stats.defence,
                    hitpoints: stats.hitpoints,
                    magic: stats.magic,
                    damagetype,
                    category: null,
                    strength: stats.strength,
                    ranged: stats.ranged,
                    attackbonus,
                    rangeattack,
                    magicattack,
                    stabdefence,
                    slashdefence,
                    crushdefence,
                    magicdefence,
                    rangedefence,
                    strengthbonus,
                    rangebonus,
                    max_dealt,
                    undead: null,
                    attackrate,
                    poison_severity,
                    forcemulti,
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
            else if (key === 'category') current.category = value;
            else if (key === 'attack') current.attack = requiredCombatInt(value, `${current.name}.attack`);
            else if (key === 'defence') current.defence = requiredCombatInt(value, `${current.name}.defence`);
            else if (key === 'hitpoints') current.hitpoints = requiredCombatInt(value, `${current.name}.hitpoints`);
            else if (key === 'magic') current.magic = requiredCombatInt(value, `${current.name}.magic`);
            else if (key === 'strength') current.strength = requiredCombatInt(value, `${current.name}.strength`);
            else if (key === 'ranged') current.ranged = requiredCombatInt(value, `${current.name}.ranged`);
            else if (key === 'param') {
                const comma = value.indexOf(',');
                if (comma <= 0) continue;
                const param = value.slice(0, comma);
                const rawValue = value.slice(comma + 1);
                current.params.set(param, rawValue);
                if (param === 'damagetype') current.damagetype = rawValue;
                else if (param === 'attackbonus') current.attackbonus = requiredParamInt(rawValue, `${current.name}.attackbonus`);
                else if (param === 'rangeattack') current.rangeattack = requiredParamInt(rawValue, `${current.name}.rangeattack`);
                else if (param === 'magicattack') current.magicattack = requiredParamInt(rawValue, `${current.name}.magicattack`);
                else if (param === 'stabdefence') current.stabdefence = requiredParamInt(rawValue, `${current.name}.stabdefence`);
                else if (param === 'slashdefence') current.slashdefence = requiredParamInt(rawValue, `${current.name}.slashdefence`);
                else if (param === 'crushdefence') current.crushdefence = requiredParamInt(rawValue, `${current.name}.crushdefence`);
                else if (param === 'magicdefence') current.magicdefence = requiredParamInt(rawValue, `${current.name}.magicdefence`);
                else if (param === 'rangedefence') current.rangedefence = requiredParamInt(rawValue, `${current.name}.rangedefence`);
                else if (param === 'strengthbonus') current.strengthbonus = requiredParamInt(rawValue, `${current.name}.strengthbonus`);
                else if (param === 'rangebonus') current.rangebonus = requiredParamInt(rawValue, `${current.name}.rangebonus`);
                else if (param === 'undead') current.undead = parseParamInt(rawValue, `${current.name}.undead`);
                else if (param === 'poison_severity') current.poison_severity = requiredParamInt(rawValue, `${current.name}.poison_severity`);
                else if (param === 'npc_forcemulti') current.forcemulti = requiredParamInt(rawValue, `${current.name}.npc_forcemulti`) !== 0;
                else if (param === 'max_dealt') current.max_dealt = requiredParamInt(rawValue, `${current.name}.max_dealt`);
                else if (param === 'attackrate') {
                    const rate = requiredParamInt(rawValue, `${current.name}.attackrate`);
                    if (rate < 0 || rate > 255) {
                        throw new Error(`npc_names: ${current.name}.attackrate is out of range: ${rawValue}`);
                    }
                    current.attackrate = rate;
                }
            }
        }
    }
    return {
        blocks,
        files,
        stats,
        combatDefaults,
    };
}

type NpcCombatTransform = { kind: 'known'; npc_id: number } | { kind: 'unknown' };

function resolveNpcCombatTransforms(
    pack: ReadonlyMap<string, number>,
    blocks: ReadonlyMap<string, NpcBlock>,
    scripts: CombatScripts,
) {
    const sources = new Map<string, NpcCombatTransformTarget | null>();
    for (const config of pack.keys()) {
        sources.set(config, extractNpcCombatTransformTarget(config, blocks.get(config)?.category ?? null, scripts, blocks.get(config)?.params));
    }
    const resolved = new Map<string, NpcCombatTransform | null>();
    const active = new Set<string>();
    const resolve = (config: string): NpcCombatTransform | null => {
        if (resolved.has(config)) return resolved.get(config)!;
        const target = sources.get(config) ?? null;
        if (!target) return null;
        if (target.kind === 'unknown') {
            resolved.set(config, target);
            return target;
        }
        if (active.has(config)) return { kind: 'unknown' };
        active.add(config);
        const targetId = pack.get(target.config);
        let result: NpcCombatTransform;
        if (targetId === undefined) {
            result = { kind: 'unknown' };
        } else {
            const next = resolve(target.config);
            result = next?.kind === 'unknown' ? next : next ?? { kind: 'known', npc_id: targetId };
        }
        active.delete(config);
        resolved.set(config, result);
        return result;
    };
    for (const config of pack.keys()) resolve(config);
    return resolved;
}



export function extractNpcNamesFacts(
    content: string,
    revision: number,
    engineNpcTypeSource: string,
    scripts: CombatScripts = parseCombatScripts(content),
    maxHits = buildSpellMaxHits(content),
) {
    if (revision !== 274 && revision !== 289 && revision !== 377) throw new Error(`npc_names: unsupported revision ${revision}`);
    const pack = parsePack(requireGatherText(content, 'pack/npc.pack'));
    if (pack.size === 0) throw new Error('npc_names: empty pack/npc.pack');
    const { blocks, files, stats, combatDefaults } = parseNpcFiles(content, revision, engineNpcTypeSource);
    const combatTransforms = resolveNpcCombatTransforms(pack, blocks, scripts);
    const combatNpcs: CombatNpcSource[] = [];
    const rows = [...pack.entries()]
        .sort((a, b) => a[1] - b[1])
        .map(([config, id]) => {
            const block = blocks.get(config);
            // Stat levels use parsed NpcType.ts defaults; tether defaults remain revision-specific.
            const wanderrange = block?.wanderrange ?? 5;
            let maxrange = block?.maxrange ?? (revision === 274 ? 7 : -1);
            if (revision !== 274) {
                if (maxrange === -1) maxrange = wanderrange + 2;
                maxrange = Math.max(maxrange, wanderrange);
            }
            const combat = extractNpcCombatFacts(config, block?.category ?? null, scripts, maxHits, revision);
            const poisonSeverityFact = block?.poison_severity ?? combatDefaults.poison_severity;
            const ordinaryMelee = combat.ordinary_melee
                && block !== undefined
                && block.strength !== null
                && block.strength >= 0
                && block.strengthbonus !== null
                && block.attackrate !== null
                && block.attackrate > 0
                && !block.forcemulti
                && poisonSeverityFact === 0;
            // The shade attack is a shared melee roll with the NPC's resolved stats; it has no
            // explicit-parameter requirement because the shade rows carry content defaults.
            const resolvedStrength = block?.strength ?? stats.strength;
            const resolvedAttackRate = block?.attackrate ?? combatDefaults.attackrate;
            const shadeMelee = combat.shade_melee
                && resolvedStrength !== null
                && resolvedStrength >= 0
                && (block?.strengthbonus ?? combatDefaults.strengthbonus) !== null
                && resolvedAttackRate !== null
                && resolvedAttackRate > 0
                && !(block?.forcemulti ?? combatDefaults.forcemulti)
                && poisonSeverityFact === 0;
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
                combat_transform: combatTransforms.get(config) ?? null,
                display: block?.display ?? null,
                ops: block?.ops ?? [],
                size: block?.size ?? 1,
                wanderrange,
                maxrange,
                attackrange: block?.attackrange ?? 0,
                huntrange: block?.huntrange ?? 0,
                vislevel: block?.vislevel ?? 0,
                headicon: block?.headicon ?? undefined,
                attack: block?.attack ?? stats.attack,
                defence: block?.defence ?? stats.defence,
                hitpoints: block?.hitpoints ?? stats.hitpoints,
                magic: block?.magic ?? stats.magic,
                damagetype: block?.damagetype ?? combatDefaults.damagetype,
                strength: block?.strength ?? stats.strength,
                ranged: block?.ranged ?? stats.ranged,
                attackbonus: block?.attackbonus ?? combatDefaults.attackbonus,
                rangeattack: block?.rangeattack ?? combatDefaults.rangeattack,
                magicattack: block?.magicattack ?? combatDefaults.magicattack,
                stabdefence: block?.stabdefence ?? combatDefaults.stabdefence,
                slashdefence: block?.slashdefence ?? combatDefaults.slashdefence,
                crushdefence: block?.crushdefence ?? combatDefaults.crushdefence,
                magicdefence: block?.magicdefence ?? combatDefaults.magicdefence,
                rangedefence: block?.rangedefence ?? combatDefaults.rangedefence,
                strengthbonus: block?.strengthbonus ?? combatDefaults.strengthbonus,
                rangebonus: block?.rangebonus ?? combatDefaults.rangebonus,
                max_dealt: block?.max_dealt ?? combatDefaults.max_dealt,
                undead: block?.undead ?? null,
                ap_attack: combat.ap_attack,
                attack_kind: combat.attack_kind,
                forced_max_hit: combat.forced_max_hit,
                dragonfire: combat.dragonfire,
                attackrate: block?.attackrate ?? combatDefaults.attackrate,
                poison_severity: block?.poison_severity ?? combatDefaults.poison_severity,
                forcemulti: block?.forcemulti ?? combatDefaults.forcemulti,
                bespoke: combat.bespoke && !ordinaryMelee && !shadeMelee,
                counter_protect: combat.counter_protect,
                combat_vars_on_hit: shadeMelee,
            };
        });
    if (rows.length === 0) throw new Error('npc_names: no extracted rows');
    if (!rows.some((row) => row.config === 'khazard_warlord' && row.id === 477)) {
        throw new Error('npc_names: missing khazard_warlord pack join');
    }
    return { rows, files, combatNpcs, combat_facts_version: NPC_COMBAT_FACTS_VERSION };
}
