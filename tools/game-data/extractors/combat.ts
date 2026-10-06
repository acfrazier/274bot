import fs from 'node:fs';
import path from 'node:path';
import { parsePack, parseRows, requireGatherText, walkContentFiles } from './common.ts';

export const STYLE = {
    MELEE: 0x01,
    RANGED: 0x02,
    MAGIC: 0x04,
    DRAGONFIRE: 0x08,
} as const;

export type NpcAttackKind = 'ranged' | 'magic' | 'mixed';
export type DragonfireKind = 'elvarg' | 'chromatic' | 'metal' | 'other';
export type CombatNpcSource = {
    config: string;
    category: string | null;
    params: ReadonlyMap<string, string>;
    attack_kind: NpcAttackKind | null;
    dragonfire: DragonfireKind | null;
};
export type CombatNpcFacts = {
    ap_attack: boolean;
    attack_kind: NpcAttackKind | null;
    forced_max_hit: number | null;
    dragonfire: DragonfireKind | null;
    bespoke: boolean;
    counter_protect: boolean;
};

type ScriptSection = { kind: string; target: string | null; body: string };
type ProcedureBody = { key: string; kind: string; body: string };
type CombatScript = {
    relative: string;
    dragonResist: boolean;
    triggers: ScriptSection[];
    procedures: Map<string, ProcedureBody[]>;
};
export type CombatScripts = {
    files: CombatScript[];
    procedures: Map<string, ProcedureBody[]>;
    triggers: Map<string, { script: CombatScript; section: ScriptSection }[]>;
};

type CombatObject = {
    id: number;
    category?: number;
    params?: ReadonlyMap<number, number | string> | null;
};
type CombatDbRow = { name: string; values: Record<string, string[][]> };

const AI_KINDS = new Set(['ai_applayer2', 'ai_opplayer2', 'ai_queue1']);

function stripComments(text: string) {
    let out = '';
    let string = false;
    let escaped = false;
    let lineComment = false;
    let blockComment = false;
    for (let i = 0; i < text.length; i++) {
        const c = text[i]!;
        const next = text[i + 1];
        if (lineComment) {
            if (c === '\n') {
                lineComment = false;
                out += c;
            } else {
                out += ' ';
            }
            continue;
        }
        if (blockComment) {
            if (c === '*' && next === '/') {
                out += '  ';
                i++;
                blockComment = false;
            } else {
                out += c === '\n' ? '\n' : ' ';
            }
            continue;
        }
        if (string) {
            out += c;
            if (escaped) escaped = false;
            else if (c === '\\') escaped = true;
            else if (c === '"') string = false;
            continue;
        }
        if (c === '"') {
            string = true;
            out += c;
        } else if (c === '/' && next === '/') {
            lineComment = true;
            out += '  ';
            i++;
        } else if (c === '/' && next === '*') {
            blockComment = true;
            out += '  ';
            i++;
        } else {
            out += c;
        }
    }
    return out;
}

function parseSections(text: string) {
    const sections: ScriptSection[] = [];
    let current: { kind: string; target: string | null; lines: string[] } | null = null;
    const flush = () => {
        if (current) sections.push({ kind: current.kind, target: current.target, body: current.lines.join('\n') });
    };
    for (const line of text.split(/\r?\n/)) {
        const header = /^\s*\[\s*([A-Za-z0-9_]+)(?:\s*,\s*([^\]]+))?\]\s*(.*)$/.exec(line);
        if (!header) {
            if (current) current.lines.push(line);
            continue;
        }
        flush();
        current = { kind: header[1]!, target: header[2]?.trim() ?? null, lines: header[3] ? [header[3]] : [] };
    }
    flush();
    return sections;
}

export function parseCombatScripts(content: string): CombatScripts {
    const files: CombatScript[] = [];
    const procedures = new Map<string, ProcedureBody[]>();
    const triggers = new Map<string, { script: CombatScript; section: ScriptSection }[]>();
    for (const file of walkContentFiles(path.join(content, 'scripts'), '.rs2')) {
        const relative = path.relative(content, file).split(path.sep).join('/');
        const code = stripComments(fs.readFileSync(file, 'utf8'));
        const sections = parseSections(code);
        const localProcedures = new Map<string, ProcedureBody[]>();
        for (const section of sections) {
            if ((section.kind === 'proc' || section.kind === 'label') && section.target) {
                const procedure = { key: `${relative}:${section.kind}:${section.target}:${localProcedures.get(section.target)?.length ?? 0}`, kind: section.kind, body: section.body };
                (localProcedures.get(section.target) ?? (localProcedures.set(section.target, []), localProcedures.get(section.target)!)).push(procedure);
                (procedures.get(section.target) ?? (procedures.set(section.target, []), procedures.get(section.target)!)).push(procedure);
            }
        }
        const script: CombatScript = {
            relative,
            dragonResist: code.includes('%dragonresist'),
            triggers: sections.filter((section) => AI_KINDS.has(section.kind)),
            procedures: localProcedures,
        };
        files.push(script);
        for (const section of script.triggers) {
            if (section.target === null) continue;
            (triggers.get(section.target) ?? (triggers.set(section.target, []), triggers.get(section.target)!)).push({ script, section });
        }
    }
    return { files, procedures, triggers };
}

function matchingTriggers(config: string, category: string | null, scripts: CombatScripts) {
    return [
        ...(scripts.triggers.get(config) ?? []),
        ...(category === null ? [] : scripts.triggers.get(`_${category}`) ?? []),
    ];
}


function referencedProcedures(body: string) {
    const names = new Set<string>();
    for (const match of body.matchAll(/(?:~|@)([A-Za-z][A-Za-z0-9_]*)\s*(?=\(|;|$)/g)) names.add(match[1]!);
    for (const match of body.matchAll(/\b(?:gosub|gosub_with_params|goto|jump)\s*\(\s*([A-Za-z][A-Za-z0-9_]*)/g)) names.add(match[1]!);
    return names;
}

const GENERIC_ATTACK_PROCEDURES = new Set([
    'npc_default_attack',
    'npc_meleeattack',
    'npc_rangeattack',
    'npc_cast_spell',
    'npc_cast_spell_with_forced_max_hit',
    'npc_magicattack',
    'npc_default_retaliate_ap',
]);

function expandSections(body: string, script: CombatScript, scripts: CombatScripts, skipGenericAttack = false) {
    const expanded = [{ body, kind: 'trigger' }];
    const seen = new Set<string>();
    const visit = (current: string, depth: number) => {
        if (depth >= 16) return;
        for (const name of referencedProcedures(current)) {
            if (skipGenericAttack && GENERIC_ATTACK_PROCEDURES.has(name)) continue;
            const candidates = script.procedures.get(name) ?? scripts.procedures.get(name) ?? [];
            for (const procedure of candidates) {
                if (seen.has(procedure.key)) continue;
                seen.add(procedure.key);
                expanded.push({ body: procedure.body, kind: procedure.kind });
                visit(procedure.body, depth + 1);
            }
        }
    };
    visit(body, 0);
    return expanded;
}

function expandBody(body: string, script: CombatScript, scripts: CombatScripts, skipGenericAttack = false) {
    return expandSections(body, script, scripts, skipGenericAttack).map((section) => section.body).join('\n');
}

const SHARED_ATTACK_PROCEDURES = new Set([
    'npc_default_attack',
    'npc_meleeattack',
    'npc_rangeattack',
    'npc_cast_spell',
    'npc_cast_spell_with_forced_max_hit',
    'npc_magicattack',
    'npc_default_retaliate',
    'npc_default_retaliate_ap',
]);

function isSharedAttackBody(body: string) {
    let found = false;
    const call = /(?:~([A-Za-z][A-Za-z0-9_]*)\s*(?:\([^;]*\))?\s*;?|gosub(?:_with_params)?\s*\(\s*([A-Za-z][A-Za-z0-9_]*)\s*(?:,[^)]*)?\)\s*;?)/g;
    const rest = body.replace(call, (source, tildeName: string | undefined, gosubName: string | undefined) => {
        if (!SHARED_ATTACK_PROCEDURES.has(tildeName ?? gosubName ?? '')) return source;
        found = true;
        return '';
    });
    return found && /^[\s{};]*$/.test(rest);
}

function dragonfireKind(relative: string): DragonfireKind {
    const source = relative.replace(/\\/g, '/');
    if (source.endsWith('quests/quest_dragon/scripts/elvarg.rs2')) return 'elvarg';
    if (source.endsWith('npc/scripts/dragon.rs2')) return 'chromatic';
    if (source.endsWith('areas/area_karamja/scripts/metal_dragon.rs2')) return 'metal';
    return 'other';
}

function spellMaxHits(content: string) {
    const maxHits = new Map<string, number>();
    for (const relative of [
        'scripts/skill_combat/configs/magic/magic_combat_spells.dbrow',
        'scripts/skill_magic/configs/magic_spells.dbrow',
    ]) {
        const file = path.join(content, relative);
        if (!fs.existsSync(file)) continue;
        for (const row of parseRows(fs.readFileSync(file, 'utf8'))) {
            const rawSpell = row.values.spell?.[0]?.[0];
            const rawMax = row.values.maxhit?.[0]?.[0];
            if (!rawSpell || rawMax === undefined) continue;
            const spell = rawSpell.replace(/^\^/, '');
            const maxHit = Number(rawMax);
            if (!Number.isInteger(maxHit)) throw new Error(`${relative}/${row.name}: invalid maxhit ${rawMax}`);
            maxHits.set(spell, Math.max(maxHits.get(spell) ?? Number.MIN_SAFE_INTEGER, maxHit));
        }
    }
    return maxHits;
}

function bodyAttackFacts(body: string, maxHits: ReadonlyMap<string, number>) {
    const ranged = /~(?:npc_rangeattack|dagannoth_rangeattack)\s*\(/.test(body) || /~(?:npc_rangeattack|dagannoth_rangeattack)\s*;/.test(body);
    const magic = /~(?:npc_cast_spell(?:_with_forced_max_hit)?|npc_magicattack)\s*\(/.test(body) || /~(?:npc_cast_spell(?:_with_forced_max_hit)?|npc_magicattack)\s*;/.test(body);
    const forced: number[] = [];
    for (const match of body.matchAll(/~npc_cast_spell_with_forced_max_hit\s*\(\s*\^?([A-Za-z0-9_]+)\s*,\s*(-?\d+)\s*,\s*(-?\d+)/g)) {
        forced.push(Number(match[3]));
    }
    for (const match of body.matchAll(/~dagannoth_rangeattack\s*\(\s*(-?\d+)\s*,\s*\^(true|false)/g)) {
        const hit = Number(match[1]);
        forced.push(match[2] === 'true' ? hit * 2 : hit);
    }
    for (const match of body.matchAll(/~npc_cast_spell\s*\(\s*\^?([A-Za-z0-9_]+)\s*,/g)) {
        const hit = maxHits.get(match[1]!);
        if (hit !== undefined) forced.push(hit);
    }
    return { ranged, magic, forcedMaxHit: forced.length ? Math.max(...forced) : null };
}

export function extractNpcCombatFacts(config: string, category: string | null, scripts: CombatScripts, maxHits: ReadonlyMap<string, number>): CombatNpcFacts {
    const matched = matchingTriggers(config, category, scripts);
    const relevant = matched.filter(({ section }) => AI_KINDS.has(section.kind));
    const apTriggers = relevant.filter(({ section }) => section.kind === 'ai_applayer2');
    const opTriggers = relevant.filter(({ section }) => section.kind === 'ai_opplayer2');
    const queueTriggers = relevant.filter(({ section }) => section.kind === 'ai_queue1');
    const apAttack = apTriggers.length > 0 || queueTriggers.some(({ section }) => referencedProcedures(section.body).has('npc_default_retaliate_ap'));
    const bespoke = relevant.some(({ section }) => !isSharedAttackBody(section.body));
    const fireScript = matched.find(({ script }) => script.dragonResist);
    const dragonfire = fireScript ? dragonfireKind(fireScript.script.relative) : null;
    const attackSections = [...apTriggers, ...opTriggers].flatMap(({ script, section }) => expandSections(section.body, script, scripts, true));
    const attacks = attackSections.map(({ body }) => bodyAttackFacts(body, maxHits));
    const hasDragonBreath = dragonfire !== null || attackSections.some(({ body }) => /\b(?:firebreath_attack|fireblast_travel|fireblast_impact)\b/.test(body));
    const sectionKinds = new Set();
    let mixedSection = false;
    for (const { body } of attackSections) {
        const facts = bodyAttackFacts(body, maxHits);
        const melee = /~(?:npc_default_attack|npc_meleeattack)\s*(?:\(|;)/.test(body);
        const kinds = [];
        if (facts.ranged) kinds.push('ranged');
        if (facts.magic) kinds.push('magic');
        if (melee || kinds.length === 0) kinds.push('melee');
        if (kinds.length > 1) mixedSection = true;
        for (const kind of kinds) sectionKinds.add(kind);
    }
    let attackKind: NpcAttackKind | null = null;
    if (hasDragonBreath || mixedSection || sectionKinds.size > 1) attackKind = 'mixed';
    else if (sectionKinds.has('ranged')) attackKind = 'ranged';
    else if (sectionKinds.has('magic')) attackKind = 'magic';
    // Protect-counter behavior is NPC-specific; shared or expanded helpers do not count.
    const counterProtect = [...apTriggers, ...opTriggers].some(({ section }) =>
        /~check_protect_prayer\s*(?:\(|;)/.test(section.body));
    const forced = attacks.flatMap((attack) => attack.forcedMaxHit === null ? [] : [attack.forcedMaxHit]);
    return { ap_attack: apAttack, attack_kind: attackKind, forced_max_hit: forced.length ? Math.max(...forced) : null, dragonfire, bespoke, counter_protect: counterProtect };
}

function sourceValueId(value: string | number | undefined, pack: ReadonlyMap<string, number>, label: string): number | null {
    if (value === undefined) return null;
    if (typeof value === 'number') return Number.isInteger(value) && value >= 0 ? value : null;
    const name = value.trim().replace(/^\^/, '');
    if (!name || name === 'null' || name === 'none' || name === '-1') return null;
    if (/^-?\d+$/.test(name)) {
        const id = Number(name);
        return id >= 0 ? id : null;
    }
    const id = pack.get(name);
    if (id === undefined) throw new Error(`${label}: missing pack entry ${name}`);
    return id;
}

function packParameterIds(content: string) {
    return parsePack(requireGatherText(content, 'pack/param.pack'));
}

function parameterValue(params: ReadonlyMap<number, number | string> | null | undefined, ids: ReadonlyMap<string, number>, name: string) {
    const id = ids.get(name);
    if (id === undefined) throw new Error(`pack/param.pack: missing ${name}`);
    return params?.get(id);
}

function addMask(map: Map<number, number>, id: number | null, mask: number) {
    if (id === null || mask === 0) return;
    map.set(id, (map.get(id) ?? 0) | mask);
}

function sourceStyleMask(body: string, triggerKind: string, npc: CombatNpcSource) {
    const fire = /\b(?:firebreath_attack|fireblast_travel|fireblast_impact|dragon_firebreath_middle_attack)\b/.test(body);
    if (fire) return STYLE.DRAGONFIRE;
    const range = /~(?:npc_rangeattack|dagannoth_rangeattack)\s*(?:\(|;)/.test(body);
    const magic = /~(?:npc_cast_spell(?:_with_forced_max_hit)?|npc_magicattack)\s*(?:\(|;)/.test(body);
    if (range && magic) return STYLE.RANGED | STYLE.MAGIC;
    if (range) return STYLE.RANGED;
    if (magic) return STYLE.MAGIC;
    if (/~(?:npc_default_attack|npc_meleeattack)\s*(?:\(|;)/.test(body)) return STYLE.MELEE;
    if (triggerKind === 'ai_opplayer2' && npc.attack_kind === 'mixed') return STYLE.MELEE | STYLE.RANGED;
    return 0;
}

function npcAnimations(body: string) {
    const names = new Set<string>();
    for (const match of body.matchAll(/\bnpc_anim\s*\(\s*\^?([A-Za-z][A-Za-z0-9_]*)\s*(?:,|\))/g)) names.add(match[1]!);
    return names;
}

type SpotanimWhere = 'attacker' | 'projectile' | 'on_us';
function makeSpotanimRows(rows: Map<number, { style: number; where: SpotanimWhere }>) {
    return [...rows.entries()].sort((a, b) => a[0] - b[0]).map(([spotanim_id, row]) => ({ spotanim_id, style: row.style, where: row.where }));
}

// Codes align with CombatTab's closed Rust enum; root IDs come from interface.pack.
const COMBAT_TAB_ROOTS = [
    ['combat_unarmed', 0],
    ['combat_heavysword', 1],
    ['combat_axe', 2],
    ['combat_blunt', 3],
    ['combat_pickaxe', 4],
    ['combat_scythe', 5],
    ['combat_hacksword', 6],
    ['combat_spear', 7],
    ['combat_spiked', 8],
    ['combat_stabsword', 9],
    ['combat_claw', 10],
    ['combat_polearm', 11],
    ['combat_bow', 12],
    ['combat_crossbow', 13],
    ['combat_thrown', 14],
    ['combat_staff_2', 15],
] as const;

type CombatTabMap = {
    byCategory: Map<string, number | null>;
    fallback: number | null;
    roots: { tab: number; root_id: number }[];
};

function weaponCombatTabs(content: string, interfaces: ReadonlyMap<string, number>): CombatTabMap {
    const source = stripComments(requireGatherText(
        content,
        'scripts/skill_combat/scripts/player/player_attackstyles.rs2',
    ));
    const rootCodes = new Map<string, number>(COMBAT_TAB_ROOTS);
    const tabForName = (name: string): number | null => {
        const tab = rootCodes.get(name);
        return tab !== undefined && interfaces.has(name) ? tab : null;
    };
    const roots: { tab: number; root_id: number }[] = [];
    for (const [name, tab] of COMBAT_TAB_ROOTS) {
        const root_id = interfaces.get(name);
        if (root_id !== undefined) roots.push({ tab, root_id });
    }
    const switchBody = bracedBody(source, /\bswitch_category\s*\(\s*oc_category\s*\(\s*\$obj\s*\)\s*\)\s*\{/);
    const unarmedProcedure = parseSections(source).find(
        (section) => section.kind === 'proc'
            && section.target?.replace(/^\./, '') === 'weapon_category_tab_attack_unarmed',
    );
    const unarmedInterfaces = [...(unarmedProcedure?.body ?? '').matchAll(
        /\bif_settab\s*\(\s*\^?([A-Za-z][A-Za-z0-9_]*)\s*,/g,
    )].map((match) => match[1]!);
    const unarmedTab = unarmedInterfaces.length === 1
        ? tabForName(unarmedInterfaces[0]!)
        : null;
    const tabForCase = (body: string): number | null => {
        if (/weapon_category_tab_attack_unarmed\s*\(/.test(body)) return unarmedTab;
        const match = /(?:~\s*)?\.?\s*weapon_category_tab_attack\s*\(\s*[^,]*,\s*[^,]*,\s*\^?([A-Za-z][A-Za-z0-9_]*)\b/.exec(body);
        return match ? tabForName(match[1]!) : null;
    };
    const byCategory = new Map<string, number | null>();
    let fallback: number | null = null;
    let fallbackSeen = false;
    if (switchBody !== null) {
        for (const match of switchBody.matchAll(
            /^\s*case\s+(default|[A-Za-z][A-Za-z0-9_]*(?:\s*,\s*[A-Za-z][A-Za-z0-9_]*)*)\s*:\s*([^\r\n]*)/gm,
        )) {
            const tab = tabForCase(match[2]!);
            if (match[1] === 'default') {
                if (fallbackSeen && fallback !== tab) fallback = null;
                else fallback = tab;
                fallbackSeen = true;
                continue;
            }
            for (const category of match[1]!.split(',').map((name) => name.trim())) {
                if (byCategory.has(category) && byCategory.get(category) !== tab) {
                    byCategory.set(category, null);
                } else {
                    byCategory.set(category, tab);
                }
            }
        }
    }
    return { byCategory, fallback: fallbackSeen ? fallback : null, roots };
}
type CombatModeSemantic = 'accurate' | 'aggressive' | 'defensive' | 'controlled' | 'rapid' | 'long_range';
const MELEE_MODE_CONSTANTS = new Map<string, CombatModeSemantic>([
    ['style_melee_accurate', 'accurate'],
    ['style_melee_aggressive', 'aggressive'],
    ['style_melee_defensive', 'defensive'],
    ['style_melee_controlled', 'controlled'],
]);
const RANGED_MODE_CONSTANTS = new Map<string, CombatModeSemantic>([
    ['style_ranged_accurate', 'accurate'],
    ['style_ranged_rapid', 'rapid'],
    ['style_ranged_longrange', 'long_range'],
]);
const RANGED_MODE_CODES = new Map<CombatModeSemantic, number>([
    ['accurate', 0],
    ['rapid', 1],
    ['long_range', 2],
]);

function bracedBody(source: string, opener: RegExp) {
    const match = opener.exec(source);
    if (!match) return null;
    const open = match.index + match[0].lastIndexOf('{');
    let depth = 1;
    for (let index = open + 1; index < source.length; index++) {
        if (source[index] === '{') depth++;
        else if (source[index] === '}' && --depth === 0) return source.slice(open + 1, index);
    }
    return null;
}

function weaponStyleTables(content: string) {
    const source = stripComments(requireGatherText(content, 'scripts/skill_combat/scripts/combat.rs2'));
    const procedure = parseSections(source).find((section) => section.kind === 'proc'
        && section.target?.replace(/^\./, '') === 'combat_get_weapon_style_data');
    const switchBody = procedure && bracedBody(
        procedure.body,
        /\bswitch_category\s*\(\s*oc_category\s*\(\s*\$weapon\s*\)\s*\)\s*\{/,
    );
    const byCategory = new Map<string, string | null>();
    let fallback: string | null = null;
    let fallbackSeen = false;
    if (switchBody !== null && switchBody !== undefined) {
        for (const match of switchBody.matchAll(
            /^\s*case\s+(default|[A-Za-z][A-Za-z0-9_]*(?:\s*,\s*[A-Za-z][A-Za-z0-9_]*)*)\s*:\s*return\s*\(\s*([A-Za-z][A-Za-z0-9_]*)\s*\)\s*;/gm,
        )) {
            const table = match[2]!;
            if (match[1] === 'default') {
                if (fallbackSeen && fallback !== table) fallback = null;
                else fallback = table;
                fallbackSeen = true;
                continue;
            }
            for (const category of match[1]!.split(',').map((name) => name.trim())) {
                if (byCategory.has(category) && byCategory.get(category) !== table) byCategory.set(category, null);
                else byCategory.set(category, table);
            }
        }
    }
    return { byCategory, fallback: fallbackSeen ? fallback : null };
}

function combatModeFacts(
    content: string,
    interfaces: ReadonlyMap<string, number>,
    tabs: CombatTabMap,
    modeConstants: ReadonlyMap<string, CombatModeSemantic>,
    modeCodesBySemantic?: ReadonlyMap<CombatModeSemantic, number>,
) {
    const dbrowText = requireGatherText(content, 'scripts/skill_combat/configs/combat.dbrow');
    const dbrowRows = new Map<string, CombatDbRow | null>();
    for (const row of parseRows(dbrowText)) {
        if (dbrowRows.has(row.name)) dbrowRows.set(row.name, null);
        else dbrowRows.set(row.name, row);
    }
    const dbrowSections = new Map<string, ScriptSection | null>();
    for (const section of parseSections(stripComments(dbrowText))) {
        if (section.target !== null) continue;
        if (dbrowSections.has(section.kind)) dbrowSections.set(section.kind, null);
        else dbrowSections.set(section.kind, section);
    }
    const schema = parseSections(stripComments(requireGatherText(content, 'scripts/skill_combat/configs/combat.dbtable')))
        .find((section) => section.kind === 'combat_style_table' && section.target === null);
    if (!schema || !/^\s*column=damagestyle,int,LIST\s*$/m.test(schema.body)) return { rows: [], varp: null };

    const constantText = stripComments(requireGatherText(
        content,
        'scripts/skill_combat/configs/combat_damagestyles.constant',
    ));
    const modeCodes = new Map<string, number | null>();
    for (const name of modeConstants.keys()) {
        const matches = [...constantText.matchAll(new RegExp(`^\\s*\\^${name}\\s*=\\s*(-?\\d+)\\s*$`, 'gm'))];
        const code = matches.length === 1 ? Number(matches[0]![1]) : NaN;
        modeCodes.set(name, Number.isInteger(code) && code >= 0 && code <= 255 ? code : null);
    }

    const controls = new Map<number, Map<number, number | null>>();
    const invalidTabs = new Set<number>();
    const rootTab = new Map<string, number>(COMBAT_TAB_ROOTS);
    const attackStyles = stripComments(requireGatherText(
        content,
        'scripts/skill_combat/scripts/player/player_attackstyles.rs2',
    ));
    for (const section of parseSections(attackStyles)) {
        if (section.kind !== 'if_button' || section.target === null) continue;
        const root = section.target.split(':', 1)[0]!;
        const tab = rootTab.get(root);
        if (tab === undefined) continue;
        const setters = [...section.body.matchAll(/\bset_attackstyle\s*\(\s*([^)]+?)\s*\)/g)];
        if (setters.length === 0) continue;
        if (setters.length !== 1) {
            invalidTabs.add(tab);
            continue;
        }
        const slotText = setters[0]![1]!.trim();
        const slot = /^\d+$/.test(slotText) ? Number(slotText) : -1;
        if (!Number.isInteger(slot) || slot < 0 || slot > 3) {
            invalidTabs.add(tab);
            continue;
        }
        const button = interfaces.get(section.target);
        const bySlot = controls.get(tab) ?? new Map<number, number | null>();
        if (bySlot.has(slot)) bySlot.set(slot, null);
        else bySlot.set(slot, button !== undefined && button >= 0 ? button : null);
        controls.set(tab, bySlot);
    }

    const styleTables = weaponStyleTables(content);
    const tableByTab = new Map<number, string | null>();
    const assignTable = (tab: number | null, table: string | null) => {
        if (tab === null) return;
        if (tableByTab.has(tab) && tableByTab.get(tab) !== table) tableByTab.set(tab, null);
        else tableByTab.set(tab, table);
    };
    for (const [category, table] of styleTables.byCategory) {
        if (!tabs.byCategory.has(category)) continue;
        assignTable(tabs.byCategory.get(category) ?? null, table);
    }
    assignTable(tabs.fallback, styleTables.fallback);

    const rows: { tab: number; slot: number; mode: number; button: number }[] = [];
    for (const [tab, table] of tableByTab) {
        const root = COMBAT_TAB_ROOTS.find(([, code]) => code === tab)?.[0];
        const rootId = root === undefined ? undefined : interfaces.get(root);
        if (table === null || rootId === undefined || rootId < 0 || invalidTabs.has(tab)) continue;
        const row = dbrowRows.get(table);
        const section = dbrowSections.get(table);
        const styles = row?.values.damagestyle;
        if (!row || !section || !/^\s*table=combat_style_table\s*$/m.test(section.body)
            || !styles || styles.length === 0 || styles.length > 4) continue;

        const tabRows: { tab: number; slot: number; mode: number; button: number }[] = [];
        let complete = true;
        for (let slot = 0; slot < styles.length; slot++) {
            const sourceName = styles[slot]?.[0]?.replace(/^\^/, '');
            if (!sourceName) {
                complete = false;
                break;
            }
            if (!modeConstants.has(sourceName)) {
                if (/^style_(?:ranged|magic|melee)_/.test(sourceName)) continue;
                complete = false;
                break;
            }
            const sourceMode = modeCodes.get(sourceName);
            const semanticMode = modeConstants.get(sourceName);
            const mode = sourceMode === undefined || sourceMode === null || semanticMode === undefined
                ? null
                : modeCodesBySemantic?.get(semanticMode) ?? sourceMode;
            const button = controls.get(tab)?.get(slot);
            if (mode === undefined || mode === null || button === undefined || button === null) {
                complete = false;
                break;
            }
            tabRows.push({ tab, slot, mode, button });
        }
        if (complete && tabRows.length > 0) rows.push(...tabRows);
    }
    rows.sort((a, b) => a.tab - b.tab || a.slot - b.slot);
    const varpsPath = path.join(content, 'pack/varp.pack');
    const varps = fs.existsSync(varpsPath) ? parsePack(requireGatherText(content, 'pack/varp.pack')) : new Map<string, number>();
    const modeVarp = varps.get('com_mode');
    return {
        rows,
        varp: modeVarp !== undefined && modeVarp >= 0 ? modeVarp : null,
    };
}

type RangedAmmoFamily = 'arrow' | 'ogre_arrow' | 'bolt' | 'thrown' | 'javelin';
type RangedWeaponRow = { obj_id: number; attackrange: number; levelrequire: number; ammo_family: RangedAmmoFamily };
type RangedAmmoRow = { obj_id: number; levelrequire: number; family: RangedAmmoFamily };

function rangedObjectFacts(
    content: string,
    objects: readonly CombatObject[],
    categories: ReadonlyMap<number, string>,
    paramIds: ReadonlyMap<string, number>,
) {
    const objectIds = parsePack(requireGatherText(content, 'pack/obj.pack'));
    const rangedWeapons: RangedWeaponRow[] = [];
    const rangedAmmo: RangedAmmoRow[] = [];
    const familyForCategory = new Map<string, RangedAmmoFamily>([
        ['weapon_bow', 'arrow'],
        ['weapon_crossbow', 'bolt'],
        ['weapon_thrown', 'thrown'],
        ['weapon_javelin', 'javelin'],
        ['arrows', 'arrow'],
        ['ogre_arrows', 'ogre_arrow'],
        ['bolts', 'bolt'],
    ]);
    const param = (object: CombatObject, name: string): number => {
        const value = parameterValue(object.params, paramIds, name);
        // ParamType.defaultInt is -1 for an omitted levelrequire; the u8 fact uses zero for no minimum level.
        if (name === 'levelrequire' && (value === undefined || value === -1)) return 0;
        if (typeof value !== 'number' || !Number.isInteger(value) || value < 0 || value > 255) {
            throw new Error(`obj ${object.id}: ${name} is missing or out of range`);
        }
        return value;
    };
    const ogreBow = objectIds.get('ogre_bow');
    if (ogreBow === undefined) throw new Error('pack/obj.pack lacks ogre_bow');
    for (const object of objects) {
        if (object.category === undefined) continue;
        const category = categories.get(object.category);
        if (category === undefined) continue;
        const family = familyForCategory.get(category);
        if (!family) continue;
        if (category.startsWith('weapon_')) {
            const ammoFamily = object.id === ogreBow ? 'ogre_arrow'
                : family === 'javelin' || family === 'thrown' ? family
                    : family === 'bolt' ? 'bolt' : 'arrow';
            rangedWeapons.push({
                obj_id: object.id,
                attackrange: param(object, 'attackrange'),
                levelrequire: param(object, 'levelrequire'),
                ammo_family: ammoFamily,
            });
        } else {
            rangedAmmo.push({
                obj_id: object.id,
                levelrequire: param(object, 'levelrequire'),
                family,
            });
        }
    }
    rangedWeapons.sort((a, b) => a.obj_id - b.obj_id);
    rangedAmmo.sort((a, b) => a.obj_id - b.obj_id);
    return { rangedWeapons, rangedAmmo };
}

export function extractCombatStyleFacts(content: string, objects: readonly CombatObject[], npcs: readonly CombatNpcSource[], scripts: CombatScripts) {
    const seqPack = parsePack(requireGatherText(content, 'pack/seq.pack'));
    const spotanimPack = parsePack(requireGatherText(content, 'pack/spotanim.pack'));
    const categoryPack = parsePack(requireGatherText(content, 'pack/category.pack'));
    const interfacePack = parsePack(requireGatherText(content, 'pack/interface.pack'));
    const combatTabs = weaponCombatTabs(content, interfacePack);
    const meleeModes = combatModeFacts(content, interfacePack, combatTabs, MELEE_MODE_CONSTANTS);
    const rangedModes = combatModeFacts(content, interfacePack, combatTabs, RANGED_MODE_CONSTANTS, RANGED_MODE_CODES);
    const paramIds = packParameterIds(content);
    const categoryNames = new Map([...categoryPack.entries()].map(([name, id]) => [id, name]));
    const ranged = rangedObjectFacts(content, objects, categoryNames, paramIds);
    const sequences = new Map<number, number>();
    const spotanims = new Map<number, { style: number; where: SpotanimWhere }>();
    const weaponStyles: { obj_id: number; style: number; attackrate: number; category: number; tab: number | null }[] = [];
    const seqId = (value: string | number | undefined, label: string) => sourceValueId(value, seqPack, label);
    const spotId = (value: string | number | undefined, label: string) => sourceValueId(value, spotanimPack, label);
    const addSpot = (id: number | null, style: number, where: SpotanimWhere, label: string) => {
        if (id === null) return;
        const existing = spotanims.get(id);
        if (existing && existing.where !== where) throw new Error(`${label}: spotanim ${id} has conflicting locations ${existing.where}/${where}`);
        spotanims.set(id, { style: (existing?.style ?? 0) | style, where });
    };
    const paramToSequence: [string, number][] = [
        ['stabattack_anim', STYLE.MELEE],
        ['slashattack_anim', STYLE.MELEE],
        ['crushattack_anim', STYLE.MELEE],
        ['rangeattack_anim', STYLE.RANGED],
    ];
    for (const object of objects) {
        for (const [name, mask] of paramToSequence) addMask(sequences, seqId(parameterValue(object.params, paramIds, name), `obj ${object.id} ${name}`), mask);
        addSpot(spotId(parameterValue(object.params, paramIds, 'proj_launch'), `obj ${object.id} proj_launch`), STYLE.RANGED, 'attacker', `obj ${object.id}`);
        addSpot(spotId(parameterValue(object.params, paramIds, 'proj_travel'), `obj ${object.id} proj_travel`), STYLE.RANGED, 'projectile', `obj ${object.id}`);
        if (object.category === undefined || object.category < 0) continue;
        const category = categoryNames.get(object.category);
        if (!category) throw new Error(`obj ${object.id}: category ${object.category} is absent from pack/category.pack`);
        if (!category.startsWith('weapon_')) continue;
        const categoryCode = category === 'weapon_bow' ? 1 : category === 'weapon_crossbow' ? 2 : category === 'weapon_thrown' ? 3 : category === 'weapon_javelin' ? 4 : category === 'weapon_staff' ? 5 : 0;
        const style = categoryCode >= 1 && categoryCode <= 4 ? STYLE.RANGED : categoryCode === 5 ? STYLE.MAGIC : STYLE.MELEE;
        const rawRate = parameterValue(object.params, paramIds, 'attackrate');
        if (rawRate !== undefined && typeof rawRate !== 'number') throw new Error(`obj ${object.id}: attackrate is not numeric`);
        const attackrate = rawRate ?? 4;
        if (!Number.isInteger(attackrate) || attackrate < 0 || attackrate > 255) throw new Error(`obj ${object.id}: attackrate ${attackrate} is out of range`);
        const tab = combatTabs.byCategory.has(category)
            ? combatTabs.byCategory.get(category) ?? null
            : combatTabs.fallback;
        weaponStyles.push({ obj_id: object.id, style, attackrate, category: categoryCode, tab });
    }
    for (const npc of npcs) {
        const attackAnim = npc.params.get('attack_anim');
        const magicAnim = npc.params.get('magicattack_anim');
        addMask(sequences, seqId(attackAnim, `npc ${npc.config} attack_anim`), STYLE.MELEE | STYLE.RANGED);
        addMask(sequences, seqId(magicAnim, `npc ${npc.config} magicattack_anim`), STYLE.MAGIC);
        addSpot(spotId(npc.params.get('proj_launch'), `npc ${npc.config} proj_launch`), STYLE.RANGED, 'attacker', `npc ${npc.config}`);
        addSpot(spotId(npc.params.get('proj_travel'), `npc ${npc.config} proj_travel`), STYLE.RANGED, 'projectile', `npc ${npc.config}`);
        for (const { script, section } of matchingTriggers(npc.config, npc.category, scripts)) {
            if (section.kind !== 'ai_applayer2' && section.kind !== 'ai_opplayer2') continue;
            for (const expanded of expandSections(section.body, script, scripts, true)) {
                const mask = sourceStyleMask(expanded.body, section.kind, npc);
                for (const name of npcAnimations(expanded.body)) addMask(sequences, seqId(name, `npc ${npc.config} npc_anim`), mask);
            }
        }
    }
    const magicRows = parseRows(requireGatherText(content, 'scripts/skill_combat/configs/magic/magic_combat_spells.dbrow'));
    for (const row of magicRows) {
        for (const field of ['anim', 'staffanim']) addMask(sequences, seqId(row.values[field]?.[0]?.[0], `${row.name} ${field}`), STYLE.MAGIC);
        addSpot(spotId(row.values.spotanim_origin?.[0]?.[0], `${row.name} spotanim_origin`), STYLE.MAGIC, 'attacker', row.name);
        addSpot(spotId(row.values.spotanim_proj?.[0]?.[0], `${row.name} spotanim_proj`), STYLE.MAGIC, 'projectile', row.name);
        addSpot(spotId(row.values.spotanim_target?.[0]?.[0], `${row.name} spotanim_target`), STYLE.MAGIC, 'on_us', row.name);
    }
    for (const [name, where] of [
        ['failedspell_impact', 'on_us'],
        ['firebreath_attack', 'attacker'],
        ['fireblast_travel', 'projectile'],
        ['fireblast_impact', 'on_us'],
    ] as const) {
        const style = name === 'failedspell_impact' ? STYLE.MAGIC : STYLE.DRAGONFIRE;
        addSpot(spotId(name, `spotanim ${name}`), style, where, name);
    }
    for (const name of ['human_unarmedpunch', 'human_unarmedkick']) {
        const id = seqPack.get(name);
        if (id === undefined) throw new Error(`pack/seq.pack: missing ${name}`);
        addMask(sequences, id, STYLE.MELEE);
    }
    return {
        style_seqs: [...sequences.entries()].sort((a, b) => a[0] - b[0]).map(([seq_id, style]) => ({ seq_id, style })),
        style_spotanims: makeSpotanimRows(spotanims),
        combat_tabs: combatTabs.roots,
        weapon_styles: weaponStyles.sort((a, b) => a.obj_id - b.obj_id),
        melee_modes: meleeModes.rows,
        melee_mode_varp: meleeModes.varp,
        ranged_weapons: ranged.rangedWeapons,
        ranged_ammo: ranged.rangedAmmo,
        ranged_modes: rangedModes.rows,
        ranged_mode_varp: rangedModes.varp,
    };
}

export function buildSpellMaxHits(content: string) {
    return spellMaxHits(content);
}
