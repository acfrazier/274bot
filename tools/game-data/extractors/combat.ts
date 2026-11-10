import fs from 'node:fs';
import path from 'node:path';
import { parsePack, parseParamDefinitions, parseRows, requireGatherText, walkContentFiles } from './common.ts';
import { stripComments } from './gathering-content.ts';
import { parseBody } from './gathering-rs2.ts';

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
    ordinary_melee: boolean;
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
    wearpos?: number;
    wearpos2?: number;
    wearpos3?: number;
    params?: ReadonlyMap<number, number | string> | null;
};
type CombatDbRow = { name: string; values: Record<string, string[][]> };
type WeaponStyleMetadata = { obj_id: number; style: number; attackrate: number; category: number; tab: number | null };
type WeaponStyleFactWire = WeaponStyleMetadata & {
    style_slot: number;
    damagetype: number;
    damagestyle: number;
    style_bonus: [number, number, number];
};
type CombatStyleSlot = Pick<WeaponStyleFactWire, 'style_slot' | 'damagetype' | 'damagestyle' | 'style_bonus'>;

// 377 declares ai_queue1..20: queue1 retaliates; queue2/3 handle damage/death. Queue1 stays in the legacy trigger set.
const AI_KINDS = new Set(['ai_applayer2', 'ai_opplayer2', 'ai_queue1']);

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


function procedureReferences(body: string) {
    const names: string[] = [];
    for (const match of body.matchAll(/(?:~|@)([A-Za-z][A-Za-z0-9_]*)\s*(?=\(|;|$)/g)) names.push(match[1]!);
    for (const match of body.matchAll(/\b(?:gosub|gosub_with_params|goto|jump)\s*\(\s*([A-Za-z][A-Za-z0-9_]*)/g)) names.push(match[1]!);
    return names;
}

function referencedProcedures(body: string) {
    return new Set(procedureReferences(body));
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
        const references = referencedProcedures(current);
        for (const name of references) {
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

function isResolvedControlFlowBody(body: string, script: CombatScript, scripts: CombatScripts) {
    const transfer = /(?:@([A-Za-z][A-Za-z0-9_]*)\s*(?:\([^;]*\))?\s*;?|(?:gosub(?:_with_params)?|goto|jump)\s*\(\s*([A-Za-z][A-Za-z0-9_]*)\s*(?:,[^)]*)?\)\s*;?)/g;
    const targets = [...body.matchAll(transfer)]
        .map((match) => match[1] ?? match[2])
        .filter((name): name is string => name !== undefined);
    const rest = body.replace(transfer, '');
    return targets.length > 0
        && /^[\s{};]*$/.test(rest)
        && targets.every((name) => script.procedures.has(name) || scripts.procedures.has(name));
}

function expandBody(body: string, script: CombatScript, scripts: CombatScripts, skipGenericAttack = false) {
    return expandSections(body, script, scripts, skipGenericAttack).map((section) => section.body).join('\n');
}
export type NpcCombatTransformTarget =
    | { kind: 'known'; config: string }
    | { kind: 'unknown' };

type NpcTypeChangeCalls = { found: boolean; targets: string[]; unsupported: boolean };

function parseCallArguments(body: string, open: number): string[] | null {
    const args: string[] = [];
    let start = open + 1;
    let depth = 0;
    let string = false;
    let escaped = false;
    for (let index = start; index < body.length; index++) {
        const character = body[index]!;
        if (string) {
            if (escaped) escaped = false;
            else if (character === '\\') escaped = true;
            else if (character === '"') string = false;
            continue;
        }
        if (character === '"') {
            string = true;
        } else if (character === '(') {
            depth++;
        } else if (character === ')') {
            if (depth === 0) {
                args.push(body.slice(start, index).trim());
                return args;
            }
            depth--;
        } else if (character === ',' && depth === 0) {
            args.push(body.slice(start, index).trim());
            start = index + 1;
        }
    }
    return null;
}

function npcTypeChangeCalls(body: string): NpcTypeChangeCalls {
    const targets: string[] = [];
    let found = false;
    let unsupported = false;
    let string = false;
    let escaped = false;
    for (let index = 0; index < body.length; index++) {
        const character = body[index]!;
        if (string) {
            if (escaped) escaped = false;
            else if (character === '\\') escaped = true;
            else if (character === '"') string = false;
            continue;
        }
        if (character === '"') {
            string = true;
            continue;
        }
        if (/[A-Za-z0-9_]/.test(body[index - 1] ?? '')) continue;
        const name = body.startsWith('npc_changetype_keepall', index)
            ? 'npc_changetype_keepall'
            : body.startsWith('npc_changetype', index) ? 'npc_changetype' : null;
        if (!name) continue;
        let open = index + name.length;
        if (/[A-Za-z0-9_]/.test(body[open] ?? '')) continue;
        while (/\s/.test(body[open] ?? '')) open++;
        if (body[open] !== '(') continue;
        found = true;
        const args = parseCallArguments(body, open);
        const target = args?.[0];
        if (args?.length !== 2 || !/^[A-Za-z][A-Za-z0-9_]*$/.test(target ?? '')) {
            unsupported = true;
        } else {
            targets.push(target);
        }
        if (args) {
            let close = open + 1;
            let depth = 0;
            let inString = false;
            let inEscape = false;
            for (; close < body.length; close++) {
                const current = body[close]!;
                if (inString) {
                    if (inEscape) inEscape = false;
                    else if (current === '\\') inEscape = true;
                    else if (current === '"') inString = false;
                } else if (current === '"') inString = true;
                else if (current === '(') depth++;
                else if (current === ')' && depth-- === 0) break;
            }
            index = close;
        }
    }
    return { found, targets, unsupported };
}

function directApproachAwakeningTarget(section: ScriptSection) {
    if (section.kind !== 'ai_applayer2') return null;
    const normalized = section.body.replace(/\s+/g, ' ').trim();
    const match = /^npc_changetype\(\s*([A-Za-z][A-Za-z0-9_]*)\s*,\s*(\d+)\s*\)\s*;\s*npc_setmode\(\s*opplayer2\s*\)\s*;?$/.exec(normalized);
    if (!match) return null;
    const duration = Number(match[2]);
    if (!Number.isSafeInteger(duration) || duration < 1) return null;
    return match[1]!;
}

/** Derive only the straight-line attack-approach awakening contract; broader transforms fail closed. */
export function extractNpcCombatTransformTarget(
    config: string,
    category: string | null,
    scripts: CombatScripts,
): NpcCombatTransformTarget | null {
    const handlers = matchingTriggers(config, category, scripts)
        .filter(({ section }) => section.kind === 'ai_applayer2' || section.kind === 'ai_opplayer2');
    let found = false;
    let unsupported = false;
    const targets = new Set<string>();
    for (const { script, section } of handlers) {
        const handlerCalls = expandSections(section.body, script, scripts)
            .map(({ body }) => npcTypeChangeCalls(body));
        if (!handlerCalls.some((calls) => calls.found)) continue;
        found = true;
        const supportedTarget = directApproachAwakeningTarget(section);
        if (supportedTarget === null
            || handlerCalls.some((calls) => calls.unsupported || calls.targets.some((target) => target !== supportedTarget))) {
            unsupported = true;
        } else {
            targets.add(supportedTarget);
        }
    }
    if (!found) return null;
    if (unsupported || targets.size !== 1) return { kind: 'unknown' };
    return { kind: 'known', config: targets.values().next().value! };
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
const DEFAULT_MELEE_DISPATCH_PROCEDURES = new Set(['npc_default_attack']);
const ORDINARY_RETALIATION_PROCEDURES = new Set(['npc_default_retaliate', 'npc_default_retaliate_ap']);

function onlySharedAttackBody(body: string, allowedProcedures: ReadonlySet<string>) {
    let found = false;
    const call = /(?:~([A-Za-z][A-Za-z0-9_]*)\s*(?:\([^;]*\))?\s*;?|gosub(?:_with_params)?\s*\(\s*([A-Za-z][A-Za-z0-9_]*)\s*(?:,[^)]*)?\)\s*;?)/g;
    const rest = body.replace(call, (source, tildeName: string | undefined, gosubName: string | undefined) => {
        if (!allowedProcedures.has(tildeName ?? gosubName ?? '')) return source;
        found = true;
        return '';
    });
    return found && /^[\s{};]*$/.test(rest);
}

function isSharedAttackBody(body: string) {
    return onlySharedAttackBody(body, SHARED_ATTACK_PROCEDURES);
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

// 377 reaches helpers through gosub/jump and @label aliases; direct legacy calls stay exact for 274/289.
function bodyAttackFacts(body: string, maxHits: ReadonlyMap<string, number>, revision: number = 289) {
    let ranged: boolean;
    let magic: boolean;
    let melee: boolean;
    if (revision === 377) {
        const references = referencedProcedures(body);
        ranged = references.has('npc_rangeattack') || references.has('dagannoth_rangeattack');
        magic = ['npc_cast_spell', 'npc_cast_spell_with_forced_max_hit', 'npc_magicattack']
            .some((name) => references.has(name));
        melee = references.has('npc_default_attack') || references.has('npc_meleeattack');
    } else {
        ranged = /~(?:npc_rangeattack|dagannoth_rangeattack)\s*\(/.test(body) || /~(?:npc_rangeattack|dagannoth_rangeattack)\s*;/.test(body);
        magic = /~(?:npc_cast_spell(?:_with_forced_max_hit)?|npc_magicattack)\s*\(/.test(body)
            || /~(?:npc_cast_spell(?:_with_forced_max_hit)?|npc_magicattack)\s*;/.test(body);
        melee = /~(?:npc_default_attack|npc_meleeattack)\s*(?:\(|;)/.test(body);
    }
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
    return { ranged, magic, melee, forcedMaxHit: forced.length ? Math.max(...forced) : null };
}

const ORDINARY_MELEE_SAFE_CALLS = new Set([
    'anim',
    'gosub',
    'gosub_with_params',
    'if',
    'npc_anim',
    'npc_param',
    'npc_say',
    'npc_setmode',
    'random',
    'sound_synth',
    'switch_int',
]);

// A shared melee attack is recognized by its procedure names: exactly one
// `npc_meleeattack` and exactly one `npc_melee_maxhit` procedure must exist.
function standardMeleeRoutineExists(scripts: CombatScripts) {
    return (scripts.procedures.get('npc_meleeattack') ?? []).length === 1
        && (scripts.procedures.get('npc_melee_maxhit') ?? []).length === 1;
}

function ordinaryMeleeBodyIsSafe(
    body: string,
    script: CombatScript,
    scripts: CombatScripts,
    active = new Set<string>(),
    depth = 0,
): boolean {
    if (depth >= 16) return false;
    if (/(?<![~@])\b(?:npc_meleeattack|npc_default_attack)\s*\(/.test(body)) return false;

    const modeCalls = [...body.matchAll(/\bnpc_setmode\s*\(\s*\^?([A-Za-z][A-Za-z0-9_]*)\s*\)/g)];
    if ((body.match(/\bnpc_setmode\b/g) ?? []).length !== modeCalls.length
        || modeCalls.some((match) => !['null', 'opplayer2', 'applayer2'].includes(match[1]!))) {
        return false;
    }

    const procedureIsSafe = (name: string): boolean => {
        if (ORDINARY_MELEE_SAFE_CALLS.has(name)) return true;
        if (name === 'npc_meleeattack') {
            return !script.procedures.has(name) && standardMeleeRoutineExists(scripts);
        }
        if (name === 'npc_default_attack') {
            return !script.procedures.has(name) && defaultAttackUsesMelee(scripts);
        }
        if (GENERIC_ATTACK_PROCEDURES.has(name)) return false;

        const candidates = script.procedures.get(name) ?? [];
        if (candidates.length !== 1 || active.has(candidates[0]!.key)) return false;
        const nextActive = new Set(active);
        nextActive.add(candidates[0]!.key);
        return ordinaryMeleeBodyIsSafe(candidates[0]!.body, script, scripts, nextActive, depth + 1);
    };

    for (const name of referencedProcedures(body)) {
        if (!procedureIsSafe(name)) return false;
    }
    for (const match of body.matchAll(/\b([A-Za-z][A-Za-z0-9_]*)\s*\(/g)) {
        if (!procedureIsSafe(match[1]!)) return false;
    }
    return true;
}

function ordinaryMeleeDispatchCount(
    body: string,
    script: CombatScript,
    scripts: CombatScripts,
    active = new Set<string>(),
    depth = 0,
): number | null {
    if (depth >= 16) return null;
    let count = 0;
    for (const name of procedureReferences(body)) {
        if (ORDINARY_MELEE_SAFE_CALLS.has(name)) continue;
        if (name === 'npc_meleeattack') {
            if (script.procedures.has(name) || !standardMeleeRoutineExists(scripts)) return null;
            count += 1;
            continue;
        }
        if (name === 'npc_default_attack') {
            if (script.procedures.has(name) || !defaultAttackUsesMelee(scripts)) return null;
            count += 1;
            continue;
        }
        if (GENERIC_ATTACK_PROCEDURES.has(name)) return null;

        const candidates = script.procedures.get(name) ?? [];
        if (candidates.length !== 1 || active.has(candidates[0]!.key)) return null;
        const nextActive = new Set(active);
        nextActive.add(candidates[0]!.key);
        const nestedCount = ordinaryMeleeDispatchCount(candidates[0]!.body, script, scripts, nextActive, depth + 1);
        if (nestedCount === null) return null;
        count += nestedCount;
    }
    return count;
}

function defaultAttackUsesMelee(scripts: CombatScripts) {
    const routines = scripts.procedures.get('npc_default_attack') ?? [];
    if (routines.length !== 1 || !referencedProcedures(routines[0]!.body).has('npc_meleeattack')) return false;
    const routine = routines[0]!;
    const script = scripts.files.find((file) =>
        file.procedures.get('npc_default_attack')?.some((candidate) => candidate.key === routine.key),
    );
    return script !== undefined
        && ordinaryMeleeBodyIsSafe(routine.body, script, scripts, new Set([routine.key]))
        && ordinaryMeleeDispatchCount(routine.body, script, scripts, new Set([routine.key])) === 1;
}

function defaultOpAttackUsesMelee(scripts: CombatScripts) {
    const handlers = (scripts.triggers.get('_') ?? []).filter(({ section }) => section.kind === 'ai_opplayer2');
    return handlers.length === 1
        && onlySharedAttackBody(handlers[0]!.section.body, DEFAULT_MELEE_DISPATCH_PROCEDURES)
        && defaultAttackUsesMelee(scripts);
}

function hasModeRoute(modes: string[], target: 'applayer2' | 'opplayer2') {
    return modes.includes(target) && modes.every((mode) => mode === target || mode === 'null');
}

function ordinaryMeleeAttackHandlers(relevant: { script: CombatScript; section: ScriptSection }[], scripts: CombatScripts) {
    const attackHandlers = relevant.filter(({ section }) => section.kind === 'ai_applayer2' || section.kind === 'ai_opplayer2');
    if (attackHandlers.length === 0) return false;
    if (relevant.some(({ section }) =>
        section.kind === 'ai_queue1' && !onlySharedAttackBody(section.body, ORDINARY_RETALIATION_PROCEDURES))) {
        return false;
    }

    const hasOpHandler = attackHandlers.some(({ section }) => section.kind === 'ai_opplayer2');
    const hasApHandler = attackHandlers.some(({ section }) => section.kind === 'ai_applayer2');
    const defaultOpMelee = !hasOpHandler && defaultOpAttackUsesMelee(scripts);
    let hasMeleeDispatch = false;

    for (const { script, section } of attackHandlers) {
        if (!ordinaryMeleeBodyIsSafe(section.body, script, scripts)) return false;
        const dispatchCount = ordinaryMeleeDispatchCount(section.body, script, scripts);
        if (dispatchCount === null || dispatchCount > 1) return false;
        if (dispatchCount === 1) {
            hasMeleeDispatch = true;
            continue;
        }
        const expanded = expandSections(section.body, script, scripts, true);
        const modes = expanded.flatMap(({ body }) =>
            [...body.matchAll(/\bnpc_setmode\s*\(\s*\^?([A-Za-z][A-Za-z0-9_]*)\s*\)/g)].map((match) => match[1]!),
        );
        if (section.kind === 'ai_opplayer2' && hasApHandler && hasModeRoute(modes, 'applayer2')) continue;
        if (section.kind === 'ai_applayer2' && hasOpHandler && hasModeRoute(modes, 'opplayer2')) continue;
        if (section.kind === 'ai_applayer2' && defaultOpMelee && hasModeRoute(modes, 'opplayer2')) {
            hasMeleeDispatch = true;
            continue;
        }
        return false;
    }

    return hasMeleeDispatch && standardMeleeRoutineExists(scripts);
}


export function extractNpcCombatFacts(config: string, category: string | null, scripts: CombatScripts, maxHits: ReadonlyMap<string, number>, revision: number = 289): CombatNpcFacts {
    const matched = matchingTriggers(config, category, scripts);
    const relevant = matched.filter(({ section }) => AI_KINDS.has(section.kind));
    const apTriggers = relevant.filter(({ section }) => section.kind === 'ai_applayer2');
    const opTriggers = relevant.filter(({ section }) => section.kind === 'ai_opplayer2');
    const queueTriggers = relevant.filter(({ section }) => section.kind === 'ai_queue1');
    const apAttack = apTriggers.length > 0 || queueTriggers.some(({ section }) =>
        referencedProcedures(section.body).has('npc_default_retaliate_ap'));
    const bespokeTriggers = revision === 377 ? [...apTriggers, ...opTriggers] : relevant;
    const bespoke = bespokeTriggers.some(({ section }) => !isSharedAttackBody(section.body));
    const fireScript = matched.find(({ script }) => script.dragonResist);
    const dragonfire = fireScript ? dragonfireKind(fireScript.script.relative) : null;
    const attackSections = [...apTriggers, ...opTriggers].flatMap(({ script, section }) =>
        expandSections(section.body, script, scripts, true).map((expanded) => ({ ...expanded, script })));
    const attacks = attackSections.map(({ body }) => bodyAttackFacts(body, maxHits, revision));
    const hasDragonBreath = dragonfire !== null || attackSections.some(({ body }) => /\b(?:firebreath_attack|fireblast_travel|fireblast_impact)\b/.test(body));
    const sectionKinds = new Set<string>();
    let mixedSection = false;
    let unknownSection = false;
    for (const { body, script } of attackSections) {
        const facts = bodyAttackFacts(body, maxHits, revision);
        const kinds: string[] = [];
        if (facts.ranged) kinds.push('ranged');
        if (facts.magic) kinds.push('magic');
        // 289's fallback treated any unclassified attack section as melee.
        // 377 keeps an unclassified branch UNKNOWN instead of promoting it.
        if (facts.melee || (kinds.length === 0 && revision !== 377)) kinds.push('melee');
        if (kinds.length === 0 && revision === 377 && !isResolvedControlFlowBody(body, script, scripts)) unknownSection = true;
        if (kinds.length > 1) mixedSection = true;
        for (const kind of kinds) sectionKinds.add(kind);
    }
    let attackKind: NpcAttackKind | null = null;
    const knownMixed = hasDragonBreath || mixedSection || sectionKinds.size > 1;
    if (knownMixed) attackKind = 'mixed';
    else if (unknownSection && revision === 377) attackKind = null;
    else if (sectionKinds.has('ranged')) attackKind = 'ranged';
    else if (sectionKinds.has('magic')) attackKind = 'magic';
    // Protect-counter behavior is NPC-specific; shared or expanded helpers do not count.
    const counterProtect = [...apTriggers, ...opTriggers].some(({ section }) =>
        revision === 377
            ? referencedProcedures(section.body).has('check_protect_prayer')
            : /~check_protect_prayer\s*(?:\(|;)/.test(section.body));
    const forced = attacks.flatMap((attack) => attack.forcedMaxHit === null ? [] : [attack.forcedMaxHit]);
    const ordinaryMelee = bespoke
        && attackKind === null
        && forced.length === 0
        && dragonfire === null
        && !counterProtect
        && ordinaryMeleeAttackHandlers(relevant, scripts);
    return { ap_attack: apAttack, attack_kind: attackKind, forced_max_hit: forced.length ? Math.max(...forced) : null, dragonfire, bespoke, ordinary_melee: ordinaryMelee, counter_protect: counterProtect };
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

function sourceStyleMask(body: string, triggerKind: string, npc: CombatNpcSource, revision: number) {
    const fire = /\b(?:firebreath_attack|fireblast_travel|fireblast_impact|dragon_firebreath_middle_attack)\b/.test(body);
    if (fire) return STYLE.DRAGONFIRE;
    let range: boolean;
    let magic: boolean;
    let melee: boolean;
    if (revision === 377) {
        const references = referencedProcedures(body);
        range = references.has('npc_rangeattack') || references.has('dagannoth_rangeattack');
        magic = ['npc_cast_spell', 'npc_cast_spell_with_forced_max_hit', 'npc_magicattack']
            .some((name) => references.has(name));
        melee = references.has('npc_default_attack') || references.has('npc_meleeattack');
    } else {
        range = /~(?:npc_rangeattack|dagannoth_rangeattack)\s*(?:\(|;)/.test(body);
        magic = /~(?:npc_cast_spell(?:_with_forced_max_hit)?|npc_magicattack)\s*(?:\(|;)/.test(body);
        melee = /~(?:npc_default_attack|npc_meleeattack)\s*(?:\(|;)/.test(body);
    }
    if (range && magic) return STYLE.RANGED | STYLE.MAGIC;
    if (range) return STYLE.RANGED;
    if (magic) return STYLE.MAGIC;
    if (melee) return STYLE.MELEE;
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
export function combatConstantCodes(content: string, file: string, label: string) {
    const source = stripComments(requireGatherText(content, file));
    const codes = new Map<string, number | null>();
    for (const match of source.matchAll(/^\s*\^([A-Za-z][A-Za-z0-9_]*)\s*=\s*(-?\d+)\s*$/gm)) {
        const name = match[1]!;
        const code = Number(match[2]);
        const prior = codes.get(name);
        if (!Number.isInteger(code) || code < 0 || code > 255) {
            throw new Error(`${label}: constant ^${name} code ${code} is out of range`);
        }
        if (prior !== undefined) throw new Error(`${label}: duplicate constant ^${name}`);
        codes.set(name, code);
    }
    return codes;
}

function combatStyleBonuses(content: string) {
    const source = stripComments(requireGatherText(content, 'scripts/skill_combat/scripts/combat.rs2'));
    const procedure = parseSections(source).find((section) => section.kind === 'proc'
        && section.target?.replace(/^\./, '') === 'combat_get_damagestyle_bonuses');
    const body = procedure && bracedBody(procedure.body, /\bswitch_int\s*\(\s*\$damagestyle\s*\)\s*\{/);
    if (body === null || body === undefined) throw new Error('combat styles: missing combat_get_damagestyle_bonuses switch');

    const bonuses = new Map<string, [number, number, number] | null>();
    let fallback: [number, number, number] | null = null;
    for (const match of body.matchAll(/^\s*case\s+(default|\^[A-Za-z][A-Za-z0-9_]*(?:\s*,\s*\^[A-Za-z][A-Za-z0-9_]*)*)\s*:\s*return\s*\(([^)]*)\)\s*;/gm)) {
        const values = match[2]!.split(',').map((value) => Number(value.trim()));
        if (values.length !== 4 || values.some((value) => !Number.isInteger(value) || value < 0 || value > 255)) {
            throw new Error(`combat styles: malformed bonus tuple ${match[2]}`);
        }
        const bonus: [number, number, number] = [values[0]!, values[1]!, values[2]!];
        if (match[1] === 'default') {
            if (fallback !== null) throw new Error('combat styles: duplicate default style bonus');
            fallback = bonus;
            continue;
        }
        for (const rawName of match[1]!.split(',')) {
            const name = rawName.trim().replace(/^\^/, '');
            if (bonuses.has(name)) bonuses.set(name, null);
            else bonuses.set(name, bonus);
        }
    }
    if (fallback === null) throw new Error('combat styles: missing default style bonus');
    return { bonuses, fallback };
}

function combatStyleSlots(content: string) {
    const schema = parseSections(stripComments(requireGatherText(content, 'scripts/skill_combat/configs/combat.dbtable')))
        .filter((section) => section.kind === 'combat_style_table' && section.target === null);
    if (schema.length !== 1
        || !/^\s*column=damagestyle,int,LIST\s*$/m.test(schema[0]!.body)
        || !/^\s*column=damagetype,int,LIST\s*$/m.test(schema[0]!.body)) {
        throw new Error('combat styles: combat_style_table must declare damagestyle and damagetype lists');
    }

    const modeCodes = combatConstantCodes(content, 'scripts/skill_combat/configs/combat_damagestyles.constant', 'damagestyle');
    const damageCodes = combatConstantCodes(content, 'scripts/skill_combat/configs/combat_damagetypes.constant', 'damagetype');
    const bonuses = combatStyleBonuses(content);
    const dbrowText = requireGatherText(content, 'scripts/skill_combat/configs/combat.dbrow');
    const rows = new Map<string, CombatDbRow | null>();
    for (const row of parseRows(dbrowText)) {
        if (rows.has(row.name)) rows.set(row.name, null);
        else rows.set(row.name, row);
    }
    const sections = new Map<string, ScriptSection | null>();
    for (const section of parseSections(stripComments(dbrowText))) {
        if (section.target !== null) continue;
        if (sections.has(section.kind)) sections.set(section.kind, null);
        else sections.set(section.kind, section);
    }

    const slots = new Map<string, CombatStyleSlot[] | null>();
    for (const [name, row] of rows) {
        const section = sections.get(name);
        if (!section || !/^\s*table=combat_style_table\s*$/m.test(section.body)) continue;
        const styles = row?.values.damagestyle;
        const damageTypes = row?.values.damagetype;
        if (!row || !styles || !damageTypes || styles.length === 0 || styles.length > 4
            || damageTypes.length !== styles.length) {
            slots.set(name, null);
            continue;
        }
        const styleRows: CombatStyleSlot[] = [];
        for (let style_slot = 0; style_slot < styles.length; style_slot++) {
            const damagestyleName = styles[style_slot]?.[0]?.replace(/^\^/, '');
            const damagetypeName = damageTypes[style_slot]?.[0]?.replace(/^\^/, '');
            const damagestyle = damagestyleName === undefined ? undefined : modeCodes.get(damagestyleName);
            const damagetype = damagetypeName === undefined ? undefined : damageCodes.get(damagetypeName);
            if (damagestyleName === undefined || damagetypeName === undefined
                || damagestyle === undefined || damagestyle === null
                || damagetype === undefined || damagetype === null) {
                throw new Error(`combat styles: ${name} slot ${style_slot} has an unresolved damage type/style`);
            }
            const bonus = bonuses.bonuses.get(damagestyleName);
            if (bonus === null) throw new Error(`combat styles: ${damagestyleName} has conflicting style bonuses`);
            styleRows.push({
                style_slot,
                damagetype,
                damagestyle,
                style_bonus: bonus ?? bonuses.fallback,
            });
        }
        slots.set(name, styleRows);
    }
    return slots;
}

function uniqueWeaponStyleMetadata<T extends WeaponStyleMetadata>(rows: readonly T[], label: string): T[] {
    const byId = new Map<number, T>();
    for (const row of rows) {
        const prior = byId.get(row.obj_id);
        if (prior && (prior.style !== row.style || prior.attackrate !== row.attackrate
            || prior.category !== row.category || prior.tab !== row.tab)) {
            throw new Error(`${label}: obj ${row.obj_id} has inconsistent per-slot weapon metadata`);
        }
        if (!prior) byId.set(row.obj_id, row);
    }
    return [...byId.values()].sort((a, b) => a.obj_id - b.obj_id);
}

type CombatBonusName = 'stabattack' | 'slashattack' | 'crushattack' | 'magicattack' | 'rangeattack'
    | 'stabdefence' | 'slashdefence' | 'crushdefence' | 'magicdefence' | 'rangedefence';
const ITEM_COMBAT_BONUS_NAMES: readonly CombatBonusName[] = [
    'stabattack', 'slashattack', 'crushattack', 'magicattack', 'rangeattack',
    'stabdefence', 'slashdefence', 'crushdefence', 'magicdefence', 'rangedefence',
];


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
    revision: number,
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
    // 274/289 already treated an absent levelrequire as zero. 377 derives
    // both ranged object defaults from the selected combat.param definitions.
    const rangedParamDefaults = new Map<string, number>();
    if (revision === 377) {
        const definitions = parseParamDefinitions(requireGatherText(content, 'scripts/skill_combat/configs/combat.param'));
        for (const name of ['attackrange', 'levelrequire']) {
            const definition = definitions.get(name);
            if (definition?.type !== 'int') throw new Error(`combat.param: ${name} is not an integer parameter`);
            const defaultRaw = definition.default;
            const defaultValue = defaultRaw === undefined ? NaN : Number(defaultRaw);
            if (!Number.isInteger(defaultValue) || defaultValue < 0 || defaultValue > 255) {
                throw new Error(`combat.param: invalid ${name} default ${defaultRaw ?? 'missing'}`);
            }
            rangedParamDefaults.set(name, defaultValue);
        }
    }
    const param = (object: CombatObject, name: string): number => {
        const value = parameterValue(object.params, paramIds, name);
        if (value === undefined || value === -1) {
            const defaultValue = revision === 377
                ? rangedParamDefaults.get(name)
                : name === 'levelrequire' ? 0 : undefined;
            if (defaultValue !== undefined) return defaultValue;
        }
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

export function extractCombatStyleFacts(content: string, objects: readonly CombatObject[], npcs: readonly CombatNpcSource[], scripts: CombatScripts, includePvpV0 = true, revision: number = 289) {
    const styleTablesByCategory = weaponStyleTables(content);
    const styleSlots = combatStyleSlots(content);
    const seqPack = parsePack(requireGatherText(content, 'pack/seq.pack'));
    const spotanimPack = parsePack(requireGatherText(content, 'pack/spotanim.pack'));
    const categoryPack = parsePack(requireGatherText(content, 'pack/category.pack'));
    const interfacePack = parsePack(requireGatherText(content, 'pack/interface.pack'));
    const combatTabs = weaponCombatTabs(content, interfacePack);
    const meleeModes = combatModeFacts(content, interfacePack, combatTabs, MELEE_MODE_CONSTANTS);
    const rangedModes = combatModeFacts(content, interfacePack, combatTabs, RANGED_MODE_CONSTANTS, RANGED_MODE_CODES);
    const paramIds = packParameterIds(content);
    if (!paramIds.has('attackrate')) throw new Error('combat styles: pack/param.pack is missing attackrate');
    const rateDefinition = parseParamDefinitions(requireGatherText(content, 'scripts/skill_combat/configs/combat.param')).get('attackrate');
    const rawDefaultRate = rateDefinition?.default;
    const defaultAttackrate = rawDefaultRate !== undefined && /^-?\d+$/.test(rawDefaultRate)
        ? Number(rawDefaultRate) : Number.NaN;
    if (rateDefinition?.type?.toLowerCase() !== 'int'
        || !Number.isSafeInteger(defaultAttackrate) || defaultAttackrate < 0 || defaultAttackrate > 255) {
        throw new Error('combat styles: attackrate has no valid content default');
    }
    const categoryNames = new Map([...categoryPack.entries()].map(([name, id]) => [id, name]));
    const ranged = rangedObjectFacts(content, objects, categoryNames, paramIds, revision);
    const sequences = new Map<number, number>();
    const spotanims = new Map<number, { style: number; where: SpotanimWhere }>();
    const weaponStyles: WeaponStyleFactWire[] = [];
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
        const attackrate = rawRate ?? defaultAttackrate;
        if (!Number.isInteger(attackrate) || attackrate < 0 || attackrate > 255) throw new Error(`obj ${object.id}: attackrate ${attackrate} is out of range`);
        const tab = combatTabs.byCategory.has(category)
            ? combatTabs.byCategory.get(category) ?? null
            : combatTabs.fallback;
        const table = styleTablesByCategory.byCategory.has(category)
            ? styleTablesByCategory.byCategory.get(category) ?? null
            : styleTablesByCategory.fallback;
        if (table === null) throw new Error(`obj ${object.id}: ${category} has no combat style table`);
        const styles = styleSlots.get(table);
        if (!styles || styles.length === 0) throw new Error(`obj ${object.id}: combat style table ${table} has no usable slots`);
        for (const styleSlot of styles) {
            weaponStyles.push({
                obj_id: object.id, style, attackrate, category: categoryCode, tab,
                ...styleSlot,
            });
        }
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
                const mask = sourceStyleMask(expanded.body, section.kind, npc, revision);
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
    // PvP V0 special-attack spotanims (PVP-DESIGN-1026 §3.3): every special
    // visual. Entries that an NPC already emits (the glow-arrow pair from
    // `quest_regicide.npc:57-58`) merge with the same style and location.
    // The halberd's `spotanim_282..285` are map spotanims, not attributed;
    // the halberd attack is attributed by its animation through `style_seq`.
    const pvpSpecialSpotanims = [
        // C`skill_combat/scripts/pvp/specs/scripts/pvp_rune_thrownaxe.rs2:86`.
        ['sp_attack_chainhit_travel_spotanim', STYLE.RANGED, 'projectile'],
        // C`pvp_dragon_dagger.rs2:4`.
        ['sp_attack_puncture_spotanim', STYLE.MELEE, 'attacker'],
        // C`pvp_dragon_longsword.rs2:18`.
        ['sp_attack_cleave_spotanim', STYLE.MELEE, 'attacker'],
        // C`pvp_dragon_mace.rs2:18`.
        ['sp_attack_shatter_spotanim', STYLE.MELEE, 'attacker'],
        // C`pvp_rune_claws.rs2:17`.
        ['sp_attack_impale_spotanim', STYLE.MELEE, 'attacker'],
        // C`pvp_dragon_spear.rs2:5`.
        ['sp_attack_shove_spotanim', STYLE.MELEE, 'attacker'],
        // C`pvp_magic_shortbow.rs2:21`.
        ['sp_attack_snapshot_spotanim', STYLE.RANGED, 'attacker'],
        // C`pvp_magic_longbow.rs2:14`.
        ['sp_attack_glow_arrow_launch', STYLE.RANGED, 'attacker'],
        // C`pvp_rune_thrownaxe.rs2:4`.
        ['sp_attack_chainhit_launch_spotanim', STYLE.RANGED, 'attacker'],
        // C`pvp_magic_shortbow.rs2:24-25`; C`pvp_magic_longbow.rs2:17`.
        ['sp_attack_glow_arrow_travel', STYLE.RANGED, 'projectile'],
    ] as const;
    // A pre-V0 revision omits these rows from `style_spotanims`.
    if (includePvpV0) {
        for (const [name, style, where] of pvpSpecialSpotanims) {
            addSpot(spotId(name, `pvp special ${name}`), style, where, name);
        }
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
        weapon_styles: weaponStyles.sort((a, b) => a.obj_id - b.obj_id || a.style_slot - b.style_slot),
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

// ---- PvP V0 facts (PVP-DESIGN-1026 §3.3, §6.1) ----
//
// One row per obj with `param=specwep,^true` in the typed schema of §3.3.
// Costs and the battleaxe/Excalibur default come from the obj params and
// `skill_combat/configs/combat.param:183-185`; tabs and styles reuse
// `extractCombatStyleFacts` (no second bake); attribution ids resolve
// through the seq/spotanim packs; the §6.1 effect distinctions are encoded
// once here with per-row content citations.

export type SpecialEffectWire =
    | {
        kind: 'damage';
        hits: number;
        max: { kind: 'melee_scaled'; num: number; den: number } | { kind: 'ranged_special_basis'; level_plus: number };
        accuracy:
            | { kind: 'melee_scaled'; num: number; den: number; side_targets: { num: number; den: number } | null }
            | { kind: 'ranged_scaled'; num: number; den: number }
            | { kind: 'ranged' }
            | { kind: 'always_hits' };
        delay: { kind: 'none' } | { kind: 'next_decision_plus_one' } | { kind: 'second_hit_next_tick' };
    }
    | { kind: 'shove'; lock_ticks: number; clears_interaction: boolean }
    | { kind: 'instant'; effect: 'dragon_battleaxe' | 'excalibur' };

export type SpecialWeaponFactWire = {
    obj_id: number;
    energy: number;
    tab: number;
    style: number;
    effect: SpecialEffectWire;
    needs: { ammo: number };
    attribution: { anim: number | null; spotanim: number | null; projectile: number | null };
};

type SpecialSpec = {
    effect: SpecialEffectWire;
    // Accepted ammo units consumed from the quiver (0 for melee).
    ammo: number;
    // Attribution names resolved through the packs; absent means the script
    // shows none in that slot.
    anim?: string;
    spotanim?: string;
    projectile?: string;
};

// Effect table of §6.1. Each row cites the per-weapon script it encodes.
const SPECIAL_SPECS: Record<string, SpecialSpec> = {
    // C`skill_combat/scripts/pvp/specs/scripts/pvp_dragon_dagger.rs2:4-5,10-12,21-23`.
    dragon_dagger: {
        effect: {
            kind: 'damage', hits: 2, max: { kind: 'melee_scaled', num: 115, den: 100 },
            accuracy: { kind: 'melee_scaled', num: 115, den: 100, side_targets: null },
            delay: { kind: 'second_hit_next_tick' },
        },
        ammo: 0, anim: 'puncture', spotanim: 'sp_attack_puncture_spotanim',
    },
    // Poison variant shares the special (dispatched by obj id in
    // C`skill_combat/scripts/pvp/pvp_special_attack.rs2`).
    dragon_dagger_p: {
        effect: {
            kind: 'damage', hits: 2, max: { kind: 'melee_scaled', num: 115, den: 100 },
            accuracy: { kind: 'melee_scaled', num: 115, den: 100, side_targets: null },
            delay: { kind: 'second_hit_next_tick' },
        },
        ammo: 0, anim: 'puncture', spotanim: 'sp_attack_puncture_spotanim',
    },
    // C`pvp_dragon_longsword.rs2:5-7,16-18`.
    dragon_longsword: {
        effect: {
            kind: 'damage', hits: 1, max: { kind: 'melee_scaled', num: 125, den: 100 },
            accuracy: { kind: 'melee_scaled', num: 100, den: 100, side_targets: null },
            delay: { kind: 'none' },
        },
        ammo: 0, anim: 'cleave', spotanim: 'sp_attack_cleave_spotanim',
    },
    // C`pvp_dragon_mace.rs2:5-7,16-18`.
    dragon_mace: {
        effect: {
            kind: 'damage', hits: 1, max: { kind: 'melee_scaled', num: 150, den: 100 },
            accuracy: { kind: 'melee_scaled', num: 125, den: 100, side_targets: null },
            delay: { kind: 'none' },
        },
        ammo: 0, anim: 'shatter', spotanim: 'sp_attack_shatter_spotanim',
    },
    // C`pvp_dragon_halberd.rs2:25,31,43-49,60-62`. Side targets roll the
    // attack at 75/100 in multi-way combat only; the map spotanims
    // (`spotanim_282..285`, `:9,13,17,21`) are not attributed.
    dragon_halberd: {
        effect: {
            kind: 'damage', hits: 1, max: { kind: 'melee_scaled', num: 110, den: 100 },
            accuracy: { kind: 'melee_scaled', num: 100, den: 100, side_targets: { num: 75, den: 100 } },
            delay: { kind: 'none' },
        },
        ammo: 0, anim: 'dragon_halberd_special_attack',
    },
    // C`pvp_dragon_spear.rs2:1-8,21-30`.
    dragon_spear: {
        effect: { kind: 'shove', lock_ticks: 4, clears_interaction: true },
        ammo: 0, anim: 'shove', spotanim: 'sp_attack_shove_spotanim',
    },
    dragon_spear_p: {
        effect: { kind: 'shove', lock_ticks: 4, clears_interaction: true },
        ammo: 0, anim: 'shove', spotanim: 'sp_attack_shove_spotanim',
    },
    tbwt_dragon_spear_kp: {
        effect: { kind: 'shove', lock_ticks: 4, clears_interaction: true },
        ammo: 0, anim: 'shove', spotanim: 'sp_attack_shove_spotanim',
    },
    // C`pvp_rune_claws.rs2:5-9,14-17`.
    rune_claws: {
        effect: {
            kind: 'damage', hits: 1, max: { kind: 'melee_scaled', num: 110, den: 100 },
            accuracy: { kind: 'melee_scaled', num: 100, den: 100, side_targets: null },
            delay: { kind: 'next_decision_plus_one' },
        },
        ammo: 0, anim: 'impale', spotanim: 'sp_attack_impale_spotanim',
    },
    // C`pvp_rune_thrownaxe.rs2:3-4,83-86`.
    rune_thrownaxe: {
        effect: {
            kind: 'damage', hits: 1, max: { kind: 'ranged_special_basis', level_plus: 10 },
            accuracy: { kind: 'ranged' }, delay: { kind: 'none' },
        },
        ammo: 1, anim: 'chainhit', spotanim: 'sp_attack_chainhit_launch_spotanim', projectile: 'sp_attack_chainhit_travel_spotanim',
    },
    // C`pvp_magic_shortbow.rs2:4-8,9-15,17,21,24-25,36-38`.
    magic_shortbow: {
        effect: {
            kind: 'damage', hits: 2, max: { kind: 'ranged_special_basis', level_plus: 10 },
            accuracy: { kind: 'ranged_scaled', num: 10, den: 7 }, delay: { kind: 'none' },
        },
        ammo: 2, anim: 'snapshot', spotanim: 'sp_attack_snapshot_spotanim', projectile: 'sp_attack_glow_arrow_travel',
    },
    // C`pvp_magic_longbow.rs2:3-8,10,14,17,20-25`. No hit roll; the launch
    // anim is the current attack anim, so no fixed anim is attributed.
    magic_longbow: {
        effect: {
            kind: 'damage', hits: 1, max: { kind: 'ranged_special_basis', level_plus: 10 },
            accuracy: { kind: 'always_hits' }, delay: { kind: 'none' },
        },
        ammo: 1, spotanim: 'sp_attack_glow_arrow_launch', projectile: 'sp_attack_glow_arrow_travel',
    },
    // C`skill_combat/scripts/player/specwep.rs2:61-79`. No `sa_energy`
    // param: the 1000 default applies.
    dragon_battleaxe: {
        effect: { kind: 'instant', effect: 'dragon_battleaxe' },
        ammo: 0, anim: 'rampage', spotanim: 'sp_attackglow_red',
    },
    // C`skill_combat/scripts/player/specwep.rs2:89-105`. No `sa_energy`
    // param: the 1000 default applies.
    excalibur: {
        effect: { kind: 'instant', effect: 'excalibur' },
        ammo: 0, anim: 'sanctuary', spotanim: 'sp_attackglow_blue',
    },
};

export function extractSpecialWeaponFacts(
    content: string,
    objects: readonly CombatObject[],
    weaponStyles: readonly WeaponStyleMetadata[],
    aliasToId: ReadonlyMap<string, number>,
): SpecialWeaponFactWire[] {
    const paramIds = packParameterIds(content);
    const seqPack = parsePack(requireGatherText(content, 'pack/seq.pack'));
    const spotanimPack = parsePack(requireGatherText(content, 'pack/spotanim.pack'));
    const animId = (name: string | undefined, label: string): number | null => {
        if (name === undefined) return null;
        return sourceValueId(name, seqPack, `${label} anim`);
    };
    const spotId = (name: string | undefined, label: string): number | null => {
        if (name === undefined) return null;
        return sourceValueId(name, spotanimPack, `${label} spotanim`);
    };
    // `sa_energy` default of `combat.param:183-185` for specwep rows without
    // the param (dragon battleaxe, Excalibur).
    const paramDefaults = parseParamDefinitions(requireGatherText(content, 'scripts/skill_combat/configs/combat.param'));
    const defaultEnergy = Number(paramDefaults.get('sa_energy')?.default);
    if (!Number.isInteger(defaultEnergy) || defaultEnergy <= 0 || defaultEnergy > 1000) {
        throw new Error(`special facts: bad sa_energy default ${paramDefaults.get('sa_energy')?.default}`);
    }
    const byId = new Map(objects.map((object) => [object.id, object]));
    const styleById = new Map(uniqueWeaponStyleMetadata(weaponStyles, 'special facts').map((row) => [row.obj_id, row]));
    const rows: SpecialWeaponFactWire[] = [];
    for (const [alias, id] of [...aliasToId.entries()].sort((a, b) => a[1] - b[1])) {
        const object = byId.get(id);
        if (!object) continue;
        const specwep = parameterValue(object.params, paramIds, 'specwep');
        if (specwep !== 1 && specwep !== '1') continue;
        const spec = SPECIAL_SPECS[alias];
        if (!spec) throw new Error(`special facts: specwep obj ${alias} (${id}) has no §6.1 effect row`);
        const style = styleById.get(id);
        if (!style) throw new Error(`special facts: specwep obj ${alias} (${id}) has no weapon_styles row`);
        if (style.tab === null || style.tab === undefined) {
            throw new Error(`special facts: specwep obj ${alias} (${id}) has no combat tab`);
        }
        const rawEnergy = parameterValue(object.params, paramIds, 'sa_energy');
        const energy = rawEnergy === undefined ? defaultEnergy : Number(rawEnergy);
        if (!Number.isInteger(energy) || energy <= 0 || energy > 1000) {
            throw new Error(`special facts: obj ${alias} (${id}) has bad sa_energy ${rawEnergy}`);
        }
        rows.push({
            obj_id: id, energy, tab: style.tab, style: style.style, effect: spec.effect,
            needs: { ammo: spec.ammo },
            attribution: {
                anim: animId(spec.anim, alias), spotanim: spotId(spec.spotanim, alias),
                projectile: spotId(spec.projectile, alias),
            },
        });
    }
    const tableAliases = new Set(Object.keys(SPECIAL_SPECS));
    for (const alias of tableAliases) {
        if (!rows.some((row) => aliasToId.get(alias) === row.obj_id)) {
            throw new Error(`special facts: §6.1 row ${alias} matches no specwep obj`);
        }
    }
    return rows.sort((a, b) => a.obj_id - b.obj_id);
}

export type ItemCombatBonusWire = { obj_id: number; strengthbonus: number; rangebonus: number } & Record<CombatBonusName, number>;

export function extractItemCombatBonus(
    content: string,
    objects: readonly CombatObject[],
): ItemCombatBonusWire[] {
    const paramIds = packParameterIds(content);
    const definitions = parseParamDefinitions(requireGatherText(content, 'scripts/skill_combat/configs/combat.param'));
    const names = [...ITEM_COMBAT_BONUS_NAMES, 'strengthbonus', 'rangebonus'] as const;
    const defaults = new Map<string, number>();
    for (const name of names) {
        if (!paramIds.has(name)) throw new Error(`combat bonus: pack/param.pack is missing ${name}`);
        const definition = definitions.get(name);
        if (definition?.type?.toLowerCase() !== 'int') throw new Error(`combat bonus: ${name} is not an integer parameter`);
        const value = definition.default === undefined ? Number.NaN : Number(definition.default);
        if (!Number.isSafeInteger(value)) throw new Error(`combat bonus: ${name} has no valid content default`);
        defaults.set(name, value);
    }
    const rows: ItemCombatBonusWire[] = [];
    for (const object of objects) {
        if (![object.wearpos, object.wearpos2, object.wearpos3].some((position) => position !== undefined && position >= 0)) continue;
        const read = (name: typeof names[number], signed = false) => {
            const raw = parameterValue(object.params, paramIds, name);
            const value = raw === undefined ? defaults.get(name)! : Number(raw);
            if (!Number.isSafeInteger(value)) throw new Error(`combat bonus: obj ${object.id} has non-integer ${name} ${raw}`);
            if (signed && (value < -32768 || value > 32767)) {
                throw new Error(`combat bonus: obj ${object.id} ${name} ${value} is out of i16 range`);
            }
            return value;
        };
        const bonuses = {} as Record<CombatBonusName, number>;
        for (const name of ITEM_COMBAT_BONUS_NAMES) bonuses[name] = read(name, true);
        rows.push({
            obj_id: object.id,
            strengthbonus: read('strengthbonus'),
            rangebonus: read('rangebonus'),
            ...bonuses,
        });
    }
    return rows.sort((a, b) => a.obj_id - b.obj_id);
}

export type MeleeWeaponTierWire = { obj_id: number; tier: number; quest_gated: boolean; strength: number };
export type MeleeWeaponTierFacts = {
    rows: MeleeWeaponTierWire[];
    unknown: { obj_id: number; alias: string; reason: string }[];
};

const MELEE_TIER_FILES = [
    'scripts/levelrequire/scripts/tier1.rs2',
    'scripts/levelrequire/scripts/tier5.rs2',
    'scripts/levelrequire/scripts/tier10.rs2',
    'scripts/levelrequire/scripts/tier20.rs2',
    'scripts/levelrequire/scripts/tier30.rs2',
    'scripts/levelrequire/scripts/tier40.rs2',
    'scripts/levelrequire/scripts/tier45.rs2',
    'scripts/levelrequire/scripts/tier50.rs2',
    'scripts/levelrequire/scripts/tier60.rs2',
    'scripts/levelrequire/scripts/tier70.rs2',
] as const;

// Handlers that gate the attack level plainly (message wording aside).
const PLAIN_ATTACK_HANDLERS = new Set(['levelrequire_attack', 'levelrequire_attack_caps']);
// Handlers that add a quest varp in front of the same attack level.
const QUEST_ATTACK_HANDLERS = new Set([
    'levelrequire_zanaris_quest_attack',
    'levelrequire_heroes_quest_attack',
    'levelrequire_regicide_quest_attack_strength',
]);

function meleeTierGate(handler: string, args: string, revision: number): { tier: number; quest_gated: boolean; strength: number } {
    const parts = args.split(',').map((part) => Number(part.trim()));
    const level = parts[0];
    if (!Number.isInteger(level) || (level as number) < 0 || (level as number) > 255) {
        throw new Error(`melee tier: handler @${handler}(${args}) has a bad attack level`);
    }
    const strengthArg = (name: string): number => {
        const value = parts[1];
        if (value === undefined) {
            throw new Error(`melee tier: handler @${name}(${args}) is missing the strength level`);
        }
        if (!Number.isInteger(value) || value < 0 || value > 255) {
            throw new Error(`melee tier: handler @${name}(${args}) has a bad strength level`);
        }
        return value;
    };
    if (PLAIN_ATTACK_HANDLERS.has(handler)) return { tier: level as number, quest_gated: false, strength: 0 };
    if (handler === 'levelrequire_zanaris_quest_attack' || handler === 'levelrequire_heroes_quest_attack') {
        return { tier: level as number, quest_gated: true, strength: 0 };
    }
    if (handler === 'levelrequire_regicide_quest_attack_strength') {
        return { tier: level as number, quest_gated: true, strength: strengthArg(handler) };
    }
    if (handler === 'levelrequire_attack_and_strength') {
        return { tier: level as number, quest_gated: false, strength: strengthArg(handler) };
    }
    if (revision === 377 && handler === 'levelrequire_attack_and_strength2') {
        return { tier: level as number, quest_gated: false, strength: level as number };
    }
    if (revision === 377 && handler === 'levelrequire_mm_quest_attack') {
        return { tier: level as number, quest_gated: true, strength: 0 };
    }
    throw new Error(`melee tier: unexpected attack handler @${handler}(${args})`);
}

function literalMeleeTierGate(body: string): { tier: number; quest_gated: boolean; strength: number } | undefined {
    const nodes = parseBody(body.split('\n').map((text, line) => ({ text, line: line + 1 })));
    if (nodes.length !== 2 || nodes[0].kind !== 'if' || nodes[1].kind !== 'stmt'
        || nodes[1].text !== '~equip(last_slot)') return undefined;
    const gate = nodes[0];
    const levels = /^stat_base\(attack\)\s*<\s*(\d+)\s*\|\s*stat_base\(strength\)\s*<\s*(\d+)$/.exec(gate.cond);
    const failure = gate.then.at(-1);
    if (!levels || gate.els.length !== 0 || !failure
        || failure.kind !== 'stmt' || failure.text !== 'return'
        || !gate.then.slice(0, -1).every(node => node.kind === 'stmt' && /^mes\(.*\)$/.test(node.text))) return undefined;
    const tier = Number(levels[1]);
    const strength = Number(levels[2]);
    if (tier > 255 || strength > 255) throw new Error('melee tier: literal attack/strength gate exceeds 255');
    return { tier, quest_gated: false, strength };
}

export function extractMeleeWeaponTiers(
    content: string,
    weaponStyles: readonly WeaponStyleMetadata[],
    idToAlias: ReadonlyMap<number, string>,
    revision = 289,
): MeleeWeaponTierFacts {
    // `weapon_styles` category 0 is melee; bows, crossbows, thrown, javelins
    // and staves tier through the ranged, magic and defence gates instead.
    // Only aliases of melee weapons are gated here; every other wield row
    // (armour, staves, ammo) is out of scope and ignored, while an
    // unexpected attack gate on a melee weapon fails closed.
    const uniqueWeaponStyles = uniqueWeaponStyleMetadata(weaponStyles, 'melee tier');
    const meleeAliases = new Set<string>();
    for (const weapon of uniqueWeaponStyles) {
        if (weapon.category !== 0) continue;
        const alias = idToAlias.get(weapon.obj_id);
        if (alias !== undefined) meleeAliases.add(alias);
    }
    // Wield blocks: `[opheld2,alias]` either carries its `@handler(args);`
    // on the same line or opens a multi-line quest block (the viking rows of
    // `tier30.rs2:18-45`) whose final attack call is the tier and whose
    // `%varp < ^varp_complete` check is the quest gate.
    type WieldBlock = { alias: string; file: string; calls: { handler: string; args: string }[]; questVarp: boolean; body?: string };
    const parseBlocks = (file: string, text: string): WieldBlock[] => {
        if (revision === 377) return parseSections(stripComments(text))
            .filter(section => section.kind === 'opheld2' && section.target !== null && /^[A-Za-z][A-Za-z0-9_]*$/.test(section.target))
            .map(section => ({
                alias: section.target!, file, body: section.body,
                calls: [...section.body.matchAll(/@([A-Za-z][A-Za-z0-9_]*)\(([^)]*)\)\s*;/g)]
                    .map(call => ({ handler: call[1]!, args: call[2]! })),
                questVarp: /%\w+\s*<\s*\^\w+_complete/.test(section.body),
            }));
        const blocks: WieldBlock[] = [];
        let current: WieldBlock | null = null;
        for (const raw of text.split(/\r?\n/)) {
            const line = raw.trim();
            const header = /^\[opheld2,([A-Za-z][A-Za-z0-9_]*)\]/.exec(line);
            if (header) {
                current = { alias: header[1]!, file, calls: [], questVarp: false };
                blocks.push(current);
            }
            if (!current) continue;
            if (/%\w+\s*<\s*\^\w+_complete/.test(line)) current.questVarp = true;
            for (const call of line.matchAll(/@([A-Za-z][A-Za-z0-9_]*)\(([^)]*)\)\s*;/g)) {
                current.calls.push({ handler: call[1]!, args: call[2]! });
            }
        }
        return blocks;
    };
    const gates = new Map<string, { tier: number; quest_gated: boolean; strength: number }>();
    const unknown = new Map<string, string>();
    for (const file of MELEE_TIER_FILES) {
        const text = requireGatherText(content, file);
        for (const block of parseBlocks(file, text)) {
            if (!meleeAliases.has(block.alias)) continue;
            if (block.calls.some((call) => call.handler === 'tutorial_island_equip')) {
                // The tutorial-only guard ends in the post-tutorial attack
                // minimum (`tutorial.rs2:267-283`); wield it as that tier.
                const tutorialText = requireGatherText(content, 'scripts/tutorial/scripts/tutorial.rs2');
                const label = tutorialText.slice(tutorialText.indexOf('[label,tutorial_island_equip]'));
                const levels = [...label.matchAll(/@levelrequire_attack\((\d+),/g)].map((m) => Number(m[1]));
                if (levels.length === 0) throw new Error('melee tier: tutorial_island_equip has no attack level');
                const prior = gates.get(block.alias);
                const gate = { tier: levels[levels.length - 1]!, quest_gated: false, strength: 0 };
                if (prior && (prior.tier !== gate.tier || prior.quest_gated !== gate.quest_gated || prior.strength !== gate.strength)) {
                    throw new Error(`melee tier: conflicting wield gates for ${block.alias}`);
                }
                gates.set(block.alias, gate);
                continue;
            }
            const attackCalls = block.calls.filter((call) =>
                PLAIN_ATTACK_HANDLERS.has(call.handler) || QUEST_ATTACK_HANDLERS.has(call.handler) ||
                call.handler === 'levelrequire_attack_and_strength'
                || (revision === 377 && ['levelrequire_attack_and_strength2', 'levelrequire_mm_quest_attack'].includes(call.handler)),
            );
            if (attackCalls.length === 0) {
                if (revision === 377) {
                    const literal = literalMeleeTierGate(block.body!);
                    if (literal) gates.set(block.alias, literal);
                    else unknown.set(block.alias, `wield gate is outside the attack/strength/quest contract in ${block.file}`);
                    continue;
                }
                throw new Error(`melee tier: melee weapon ${block.alias} (${block.file}) has no attack gate`);
            }
            const last = attackCalls[attackCalls.length - 1]!;
            const gate = meleeTierGate(last.handler, last.args, revision);
            const questGated = gate.quest_gated || block.questVarp;
            const prior = gates.get(block.alias);
            if (prior && (prior.tier !== gate.tier || prior.quest_gated !== questGated || prior.strength !== gate.strength)) {
                throw new Error(`melee tier: conflicting wield gates for ${block.alias}`);
            }
            gates.set(block.alias, { tier: gate.tier, quest_gated: questGated, strength: gate.strength });
        }
    }
    const rows: MeleeWeaponTierWire[] = [];
    const unknownRows: MeleeWeaponTierFacts['unknown'] = [];
    for (const weapon of uniqueWeaponStyles) {
        if (weapon.category !== 0) continue;
        const alias = idToAlias.get(weapon.obj_id);
        const gate = alias === undefined ? undefined : gates.get(alias);
        if (revision === 377 && (alias === undefined || unknown.has(alias) || !gate)) {
            unknownRows.push({
                obj_id: weapon.obj_id, alias: alias ?? '<unmapped>',
                reason: alias === undefined ? 'selected obj alias is unavailable'
                    : unknown.get(alias) ?? 'no selected wield gate proves a tier; the legacy tier-0 fallback is not inherited',
            });
            continue;
        }
        // No tier script is tier 0 (`combat.param:81-83`), never excluded.
        rows.push({ obj_id: weapon.obj_id, tier: gate?.tier ?? 0, quest_gated: gate?.quest_gated ?? false, strength: gate?.strength ?? 0 });
    }
    return { rows: rows.sort((a, b) => a.obj_id - b.obj_id), unknown: unknownRows.sort((a, b) => a.obj_id - b.obj_id) };
}
