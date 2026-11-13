// M-296 owns full-roster extraction/resolution programs; this preserves the existing query asset.
import fs from 'node:fs';
import path from 'node:path';
import { integer, parsePack, requireGatherText, walkContentFiles } from './common.ts';
import { parseRs2Blocks, stripComment, walkContent, type Rs2Block } from './gathering-content.ts';
import { parseBody, walkStatements, type Node, type Stmt } from './gathering-rs2.ts';
import { postCompletionProgressWriters } from './prereq-preservation.ts';
const QUEST_IDENTITY_SEEDS = [
    { id: 'cook', component: 'cook' },
    { id: 'runemysteries', component: 'runemysteries' },
    { id: 'murder', component: 'murder' },
    { id: 'waterfall', component: 'waterfall' },
    { id: 'death', component: 'death' },
    { id: 'zanaris', component: 'zanaris' },
] as const;

/** Source files scanned when quest progress domains are derived. */
export function questIdentityAnalysisFiles(content: string) {
    return walkContent(content, 'scripts', ['.rs2', '.constant', '.varp']).filter(productionScriptPath);
}
function productionScriptPath(file: string) {
    return !file.split(/[\\/]/).some((segment) => segment === '_test' || segment === 'debug');
}

type QuestItemAlias = { alias: string; quantity: number | null; kind: 'inv' | 'use-site' };
type QuestSkillGate = { skill: string; level: number };
type QuestRequirements = {
    qualification: 'partial';
    skills: QuestSkillGate[];
    items: QuestItemAlias[];
    empty_must_have: boolean;
    unknown_as_satisfied: false;
};

type QuestIdentityOutputRow = {
    id: string;
    component: string;
    display: string;
    varp: string;
    varp_id: number;
    complete: number;
    quest_points: number;
    unknown_sides: string[];
    requirements: QuestRequirements;
    members: boolean;
    kind: 'quest' | 'stub' | 'miniquest';
    journal_title: string | null;
    journal_script: string | null;
};

function parseQuestColourCalls(text: string) {
    const calls: { component: string; progress: string; complete: string }[] = [];
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line.startsWith('~send_quest_progress_colour(questlist:')) continue;
        const match = /^~send_quest_progress_colour\(questlist:([A-Za-z0-9_]+),\s*([^,]+),\s*(.+)\);$/.exec(line);
        if (!match) throw new Error(`quests.rs2: malformed colour call ${line}`);
        calls.push({ component: match[1], progress: match[2].trim(), complete: match[3].trim() });
    }
    return calls;
}

function parseQuestConstants(text: string, label = 'quest.constant', allowEmpty = false) {
    const out = new Map<string, number>();
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        const match = /^\^([A-Za-z0-9_]+)\s*=\s*(-?\d+)\s*(?:\/\/.*)?$/.exec(line);
        if (!match) continue;
        if (out.has(match[1])) throw new Error(`${label}: duplicate ^${match[1]}`);
        out.set(match[1], integer(match[2], `^${match[1]}`));
    }
    if (out.size === 0 && !allowEmpty) throw new Error(`${label}: no constants`);
    return out;
}
function questConstantsFor377(content: string, base: ReadonlyMap<string, number>) {
    const values = new Map(base);
    const ambiguous = new Set<string>();
    const root = path.join(content, 'scripts/quests');
    const files = walkContentFiles(root, '.constant')
        .filter((file) => !file.includes('/_test/') && !file.includes('/_unpack/'));
    for (const file of files) {
        const label = path.relative(content, file).split(path.sep).join('/');
        const local = parseQuestConstants(fs.readFileSync(file, 'utf8'), label, true);
        for (const [name, value] of local) {
            if (ambiguous.has(name)) continue;
            const previous = values.get(name);
            if (previous !== undefined && previous !== value) {
                values.delete(name);
                ambiguous.add(name);
            } else {
                values.set(name, value);
            }
        }
    }
    return { values, ambiguous };
}


function parseQuestListText(text: string) {
    const out = new Map<string, string>();
    let section: string | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            section = line.slice(1, -1);
            continue;
        }
        if (!section || !line.startsWith('text=')) continue;
        if (out.has(section)) throw new Error(`questlist.if: duplicate text= for ${section}`);
        const display = line.slice('text='.length).trim();
        if (!display) throw new Error(`questlist.if: empty text= for ${section}`);
        out.set(section, display);
    }
    return out;
}

function scriptHas(text: string, pattern: RegExp, label: string) {
    if (!pattern.test(text)) throw new Error(label);
}

function cookRequirements(text: string): QuestRequirements {
    const aliases = ['egg', 'bucket_milk', 'pot_flour'] as const;
    for (const alias of aliases) {
        scriptHas(text, new RegExp(`inv_total\\(inv, ${alias}\\)(?![A-Za-z0-9_])`), `quest_cook.rs2: missing inv_total for ${alias}`);
        scriptHas(text, new RegExp(`inv_del\\(inv, ${alias}, 1\\)(?![A-Za-z0-9_])`), `quest_cook.rs2: missing inv_del quantity 1 for ${alias}`);
    }
    return {
        qualification: 'partial',
        skills: [],
        items: aliases.map((alias) => ({ alias, quantity: 1, kind: 'inv' as const })),
        empty_must_have: false,
        unknown_as_satisfied: false,
    };
}

function waterfallRequirements(text: string): QuestRequirements {
    scriptHas(text, /last_useitem ! rope(?![A-Za-z0-9_])/, 'quest_waterfall.rs2: missing rope use-site check');
    return {
        qualification: 'partial',
        skills: [],
        items: [{ alias: 'rope', quantity: null, kind: 'use-site' }],
        empty_must_have: false,
        unknown_as_satisfied: false,
    };
}

function zanarisRequirements(text: string): QuestRequirements {
    const skills = [
        { skill: 'woodcutting', level: 36 },
        { skill: 'crafting', level: 31 },
    ];
    for (const gate of skills) {
        scriptHas(text, new RegExp(`stat\\(${gate.skill}\\) < ${gate.level}(?!\\d)`), `quest_zanaris.rs2: missing ${gate.skill} gate ${gate.level}`);
    }
    return {
        qualification: 'partial',
        skills,
        items: [],
        empty_must_have: false,
        unknown_as_satisfied: false,
    };
}

function emptyMustHave(): QuestRequirements {
    return {
        qualification: 'partial',
        skills: [],
        items: [],
        empty_must_have: true,
        unknown_as_satisfied: false,
    };
}

function requirementsFor(id: string, content: string): QuestRequirements {
    if (id === 'cook') return cookRequirements(requireGatherText(content, 'scripts/quests/quest_cook/scripts/quest_cook.rs2'));
    if (id === 'waterfall') return waterfallRequirements(requireGatherText(content, 'scripts/quests/quest_waterfall/scripts/quest_waterfall.rs2'));
    if (id === 'zanaris') return zanarisRequirements(requireGatherText(content, 'scripts/quests/quest_zanaris/scripts/quest_zanaris.rs2'));
    return emptyMustHave();
}

function parseQuestListEntries(text: string) {
    const entries: { component: string; display: string; members: boolean; stub: boolean }[] = [];
    let section: string | null = null;
    let members = false;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            section = line.slice(1, -1);
            continue;
        }
        if (!section || !line.startsWith('text=')) continue;
        const display = line.slice('text='.length).trim();
        if (!display) throw new Error(`questlist.if: empty text= for ${section}`);
        if (display === 'FREE QUESTS:') {
            members = false;
            continue;
        }
        if (display === 'MEMBERS QUESTS:') {
            members = true;
            continue;
        }
        if (display === 'Quest Journal') continue;
        if (section.startsWith('com_') && display === 'Haunted Mine') {
            entries.push({ component: 'hauntedmine', display, members, stub: true });
            continue;
        }
        if (section.startsWith('com_')) continue;
        entries.push({ component: section, display, members, stub: false });
    }
    return entries;
}

function journalTitle(block: Rs2Block): string | null {
    const text = block.body.map(({ text }) => stripComment(text)).join('\n');
    const literal = /~quest_journal\(\s*"([^"]+)"\s*,/.exec(text);
    if (literal) return literal[1];

    // A local title is constant only when its single, unconditional literal
    // assignment precedes the call in this script block.
    const bindings = new Map<string, { order: number } | null>();
    const journal: { argument: { variable: string; order: number } | null } = { argument: null };
    let order = 0;
    walkStatements(parseBody(block.body), (stmt, conditions) => {
        order += 1;
        const assignment = /^(?:def_string\s+)?(\$[A-Za-z0-9_]+)\s*=\s*(.+)$/.exec(stmt.text);
        if (assignment) {
            const variable = assignment[1];
            bindings.set(variable, !bindings.has(variable) && conditions.length === 0 && /^"[^"]+"$/.test(assignment[2])
                ? { order }
                : null);
        }
        const call = /^~quest_journal\(\s*(\$[A-Za-z0-9_]+)\s*,/.exec(stmt.text);
        if (call && !journal.argument) journal.argument = { variable: call[1], order };
    });
    if (!journal.argument) return null;
    const { variable, order: callOrder } = journal.argument;
    const binding = bindings.get(variable);
    if (!binding || binding.order >= callOrder) return null;

    // Keep the literal's original whitespace; the structural reader normalizes
    // statement text. Ambiguous source matches are not title evidence.
    const values = [...text.matchAll(new RegExp(`\\${variable}\\s*=\\s*"([^"]+)"\\s*;`, 'g'))];
    return values.length === 1 ? values[0][1] : null;
}

function journalIndex(content: string) {
    const titles = new Map<string, { title: string; script: string }>();
    const root = `${content}/scripts/quests`;
    const walk = (dir: string) => {
        let names: string[] = [];
        try {
            names = fs.readdirSync(dir);
        } catch {
            return;
        }
        for (const name of names) {
            const full = `${dir}/${name}`;
            let stat;
            try {
                stat = fs.statSync(full);
            } catch {
                continue;
            }
            if (stat.isDirectory()) {
                walk(full);
                continue;
            }
            if (!name.endsWith('.rs2') || !name.includes('journal')) continue;
            const text = fs.readFileSync(full, 'utf8');
            for (const block of parseRs2Blocks(full.slice(content.length + 1), text)) {
                if (block.kind !== 'if_button') continue;
                const button = /^questlist:([A-Za-z0-9_]+)$/.exec(block.name);
                if (!button) continue;
                const title = journalTitle(block);
                if (title) titles.set(button[1], { title, script: block.span.file });
            }
        }
    };
    walk(root);
    return titles;
}

function firstVarpCall(calls: { component: string; progress: string; complete: string }[], component: string) {
    const found = calls.filter((call) => call.component === component);
    return found.find((call) => call.progress.startsWith('%')) ?? found[0] ?? null;
}
function questProgressFor377(calls: { progress: string }[], varpPack: ReadonlyMap<string, number>) {
    const expressions = [...new Set(calls.map((call) => call.progress))];
    const varps = [...new Set(expressions.flatMap((expression) => {
        const match = /^%([A-Za-z0-9_]+)$/.exec(expression);
        return match ? [match[1]] : [];
    }))];
    if (varps.length !== 1 || expressions.some((expression) => expression !== `%${varps[0]}` && expression !== '1')) {
        return {
            varp: expressions.length === 1 ? expressions[0] : expressions.join(' | '),
            varpId: -1,
            unknownSides: ['progress_operand'],
        };
    }
    const varp = varps[0];
    const varpId = varpPack.get(varp);
    if (varpId === undefined) return { varp, varpId: -1, unknownSides: ['varp_pack_id'] };
    return { varp, varpId, unknownSides: [] as string[] };
}

function questCompletionThreshold377(
    operand: string,
    constants: ReadonlyMap<string, number>,
    ambiguous: ReadonlySet<string>,
): number | null {
    const constant = /^\^([A-Za-z0-9_]+)$/.exec(operand);
    if (constant) return ambiguous.has(constant[1]) ? null : constants.get(constant[1]) ?? null;
    if (/^-?\d+$/.test(operand)) return integer(operand, `quest completion ${operand}`);
    const bitMask = /^sub\(\s*pow\(\s*2\s*,\s*\^([A-Za-z0-9_]+)\s*\)\s*,\s*1\s*\)$/.exec(operand);
    if (!bitMask || ambiguous.has(bitMask[1])) return null;
    const bits = constants.get(bitMask[1]);
    if (bits === undefined) return null;
    const value = 2 ** bits - 1;
    return Number.isSafeInteger(value) ? value : null;
}

function questCompletionFor377(
    component: string,
    calls: { complete: string }[],
    constants: ReadonlyMap<string, number>,
    ambiguous: ReadonlySet<string>,
) {
    if (calls.length === 0) {
        const name = `${component}_complete`;
        const complete = ambiguous.has(name) ? undefined : constants.get(name);
        return complete === undefined
            ? { complete: -1, unknownSides: ['completion_operand'] }
            : { complete, unknownSides: [] as string[] };
    }
    const values = calls.map((call) => questCompletionThreshold377(call.complete, constants, ambiguous));
    if (values.length === 0 || values.some((value) => value === null)) {
        return { complete: -1, unknownSides: ['completion_operand'] };
    }
    const unique = new Set(values);
    if (unique.size !== 1) return { complete: -1, unknownSides: ['completion_operand'] };
    return { complete: values[0]!, unknownSides: [] as string[] };
}

function questPointsFor377(
    component: string,
    calls: { complete: string }[],
    constants: ReadonlyMap<string, number>,
    ambiguous: ReadonlySet<string>,
) {
    const directName = `${component}_questpoints`;
    const direct = ambiguous.has(directName) ? undefined : constants.get(directName);
    if (direct !== undefined) return direct;
    const candidates = new Set(calls.flatMap((call) => {
        const stage = /^\^([A-Za-z0-9_]+)_complete$/.exec(call.complete);
        const name = stage && `${stage[1]}_questpoints`;
        const value = name && !ambiguous.has(name) ? constants.get(name) : undefined;
        return value === undefined ? [] : [value];
    }));
    return candidates.size === 1 ? [...candidates][0] : null;
}

function questIdentityRowsFor377(
    content: string,
    entries: { component: string; display: string; members: boolean; stub: boolean }[],
    calls: { component: string; progress: string; complete: string }[],
    constants: ReadonlyMap<string, number>,
    ambiguous: ReadonlySet<string>,
    varpPack: ReadonlyMap<string, number>,
    journals: Map<string, { title: string; script: string }>,
): QuestIdentityOutputRow[] {
    return entries.map((entry): QuestIdentityOutputRow => {
        const componentCalls = calls.filter((call) => call.component === entry.component);
        const progress = questProgressFor377(componentCalls, varpPack);
        const completion = questCompletionFor377(entry.component, componentCalls, constants, ambiguous);
        const questPoints = questPointsFor377(entry.component, componentCalls, constants, ambiguous);
        const requirements = requirementsFor(entry.component, content);
        const unknownSides = new Set([...progress.unknownSides, ...completion.unknownSides, 'requirements']);
        if (questPoints === null) unknownSides.add('quest_points');
        const journal = journals.get(entry.component);
        const incompleteIdentity = progress.unknownSides.length > 0 || completion.unknownSides.length > 0;
        return {
            id: entry.component,
            component: entry.component,
            display: entry.display,
            varp: progress.varp,
            varp_id: progress.varpId,
            complete: completion.complete,
            quest_points: questPoints ?? -1,
            unknown_sides: [...unknownSides].sort(),
            requirements,
            members: entry.members,
            kind: entry.stub || incompleteIdentity ? 'stub' : 'quest',
            journal_title: journal?.title ?? null,
            journal_script: journal?.script ?? null,
        };
    });
}

function questVarpConfigName(text: string): string {
    const names = [...text.matchAll(/^\s*\[([A-Za-z0-9_]+)\]\s*$/gm)].map((match) => match[1]);
    if (names.length !== 1) throw new Error(`quest_barcrawl.varp: expected one varp section, got ${names.length}`);
    return names[0]!;
}

function barcrawlDisplay(text: string): string {
    const names = new Set([...text.matchAll(/"[^"\r\n]*\bdoing\s+([^"\r\n]+)"/g)]
        .map((match) => match[1].replace(/[.!?]+$/, '').trim())
        .filter((name) => /barcrawl/i.test(name)));
    if (names.size !== 1) throw new Error(`barcrawl: expected one source dialogue name, got ${names.size}`);
    return [...names][0]!;
}

function barcrawlIdentityFor377(content: string, varpPack: ReadonlyMap<string, number>): QuestIdentityOutputRow {
    const label = 'quest_barcrawl.constant';
    const constants = parseQuestConstants(requireGatherText(content, `scripts/quests/quest_barcrawl/configs/${label}`), label);
    const varp = questVarpConfigName(requireGatherText(content, 'scripts/quests/quest_barcrawl/configs/quest_barcrawl.varp'));
    const complete = constants.get(`${varp}_complete`);
    if (complete === undefined) throw new Error(`${label}: missing ^${varp}_complete`);
    if (constants.get(`${varp}_not_started`) === undefined) throw new Error(`${label}: missing ^${varp}_not_started`);
    const varpId = varpPack.get(varp);
    if (varpId === undefined) throw new Error(`quest_barcrawl.varp: %${varp} missing from varp.pack`);
    const questScript = requireGatherText(content, 'scripts/quests/quest_barcrawl/scripts/quest_barcrawl.rs2');
    scriptHas(questScript, new RegExp(`%${varp}\\b`), 'quest_barcrawl.rs2: missing barcrawl varp reference');
    const outpost = requireGatherText(content, 'scripts/areas/area_barbarian_outpost/scripts/barbarian_outpost_guard.rs2');
    scriptHas(outpost, new RegExp(`%${varp}\\s*=\\s*\\^${varp}_complete\\b`), 'barbarian_outpost_guard.rs2: missing completion write');
    const bartender = requireGatherText(content, 'scripts/areas/area_ardougne_east/scripts/bartender.rs2');
    const display = barcrawlDisplay(bartender);
    return {
        id: varp,
        component: varp,
        display,
        varp,
        varp_id: varpId,
        complete,
        quest_points: -1,
        unknown_sides: ['quest_points', 'requirements'],
        requirements: emptyMustHave(),
        members: true,
        kind: 'miniquest',
        journal_title: null,
        journal_script: null,
    };
}


/** Every `[proc,<name>]` block in the selected scripts, keyed by name. Ambiguous names keep every definition. */
function procDefinitions(content: string) {
    const procs = new Map<string, Rs2Block[]>();
    for (const file of walkContentFiles(`${content}/scripts`, '.rs2')) {
        const relative = file.slice(content.length + 1);
        for (const block of parseRs2Blocks(relative, fs.readFileSync(file, 'utf8'))) {
            if (block.kind !== 'proc') continue;
            const known = procs.get(block.name);
            if (known) known.push(block);
            else procs.set(block.name, [block]);
        }
    }
    return procs;
}

/**
 * The one varp a progress proc returns when its single `return` is exactly a
 * bare `%varp` or `getbit_range(%varp, ...)`. Any other expression (a computed
 * value, two returns, or two varps) stays unresolved: a proc call is progress,
 * never a varp itself.
 */
function procProgressVarp(block: Rs2Block): string | null {
    const returns: string[] = [];
    walkStatements(parseBody(block.body), (stmt) => {
        if (/^return(?![A-Za-z0-9_$])/.test(stmt.text)) returns.push(stmt.text);
    });
    if (returns.length !== 1) return null;
    const value = /^return\s*\((.*)\)$/.exec(returns[0])?.[1] ?? /^return\s+(%[A-Za-z0-9_]+)$/.exec(returns[0])?.[1];
    if (value === undefined) return null;
    const names = new Set([...value.replace(/"[^"]*"/g, '""').matchAll(/%([A-Za-z0-9_]+)/g)].map((match) => match[1]));
    if (names.size !== 1) return null;
    const [varp] = [...names];
    if (value === `%${varp}`) return varp;
    // The call must be the whole value: its arguments (varp, start, end) hold no
    // parentheses, so `getbit_range(%foo, 0, 3) + (1)` cannot match.
    return new RegExp(`^getbit_range\\s*\\(\\s*%${varp}\\s*,[^()]*\\)$`).test(value) ? varp : null;
}

/** Resolve `~proc` to its backing varp; a missing, duplicated or ambiguous proc stays unresolved. */
function resolveProgressProc(procs: Map<string, Rs2Block[]>, operand: string) {
    const name = /^~([A-Za-z0-9_]+)/.exec(operand)?.[1];
    if (!name) return null;
    const blocks = procs.get(name);
    return blocks?.length === 1 ? procProgressVarp(blocks[0]) : null;
}

/** `^name = value` in the quest's own `scripts/quests/quest_<component>` constants; null when absent or contradictory. */
function questLocalConstant(content: string, component: string, name: string) {
    const root = `${content}/scripts/quests/quest_${component}`;
    if (!fs.existsSync(root)) return null;
    const values = new Set<number>();
    for (const file of walkContentFiles(root, '.constant')) {
        for (const raw of fs.readFileSync(file, 'utf8').split(/\r?\n/)) {
            const match = /^\^([A-Za-z0-9_]+)\s*=\s*(-?\d+)\s*(?:\/\/.*)?$/.exec(raw.trim());
            if (match && match[1] === name) values.add(integer(match[2], `^${name}`));
        }
    }
    return values.size === 1 ? [...values][0] : null;
}

type ProgressMetadata = {
    colour_authoritative: boolean;
    stage_domain: { min: number; max: number } | null;
    transmission: 'posted_by_content' | 'not_transmitted' | 'unknown';
};

function progressConstants(content: string, sourceFiles: readonly string[]) {
    const values = new Map<string, number | null>();
    for (const file of sourceFiles) {
        if (!file.endsWith('.constant')) continue;
        const text = fs.readFileSync(`${content}/${file}`, 'utf8');
        for (const raw of text.split(/\r?\n/)) {
            const match = /^\^([A-Za-z0-9_]+)\s*=\s*(-?\d+)\s*(?:\/\/.*)?$/.exec(raw.trim());
            if (!match) continue;
            const value = integer(match[2], `^${match[1]}`);
            if (!values.has(match[1])) values.set(match[1], value);
            else if (values.get(match[1]) !== value) values.set(match[1], null);
        }
    }
    return values;
}

type ProgressWriter = { block: Rs2Block; line: number; right: string; nodes: Node[] | null };

export function questProgressWriters(content: string, sourceFiles: readonly string[]) {
    const byVariable = new Map<string, ProgressWriter[]>();
    const blocks: Rs2Block[] = [];
    const add = (variable: string, block: Rs2Block, line: number, right: string, nodes: Node[] | null) => {
        const writers = byVariable.get(variable) ?? [];
        if (!writers.some((writer) => writer.block === block && writer.line === line && writer.right === right)) {
            writers.push({ block, line, right, nodes });
            byVariable.set(variable, writers);
        }
    };
    const addAssignment = (block: Rs2Block, line: number, text: string, nodes: Node[] | null) => {
        const code = text.trim().replace(/;$/, '').trim();
        const assignment = /^([%$][A-Za-z0-9_]+(?:\s*,\s*[%$][A-Za-z0-9_]+)*)\s*=\s*(.+)$/.exec(code);
        if (!assignment) return;
        // Multiple outputs do not prove which RHS value reaches this varp.
        const right = assignment[1].includes(',') ? code : assignment[2].trim();
        for (const target of assignment[1].matchAll(/%([A-Za-z0-9_]+)/g)) {
            add(target[1], block, line, right, nodes);
        }
    };
    for (const file of sourceFiles) {
        if (!file.endsWith('.rs2') || !productionScriptPath(file)) continue;
        const text = fs.readFileSync(`${content}/${file}`, 'utf8');
        for (const block of parseRs2Blocks(file, text)) {
            blocks.push(block);
            let nodes: Node[] | null = null;
            try {
                nodes = parseBody(block.body);
                walkStatements(nodes, (stmt) => addAssignment(block, stmt.line, stmt.text, nodes));
            } catch {
                // Physical assignment lines below still cover unparsed switch bodies.
            }
            for (const { line, text: raw } of block.body) {
                const code = stripComment(raw).trim().replace(/^case\s+[^:]+:\s*/, '');
                addAssignment(block, line, code, nodes);
            }
        }
    }
    return { byVariable, blocks };
}

function varpTransmission(content: string, varp: string, sourceFiles: readonly string[]): ProgressMetadata['transmission'] {
    let sawSection = false;
    let sawYes = false;
    let sawNo = false;
    for (const file of sourceFiles) {
        if (!file.endsWith('.varp')) continue;
        let section: string | null = null;
        for (const raw of fs.readFileSync(`${content}/${file}`, 'utf8').split(/\r?\n/)) {
            const line = raw.trim();
            if (!line || line.startsWith('//')) continue;
            if (line.startsWith('[') && line.endsWith(']')) {
                section = line.slice(1, -1);
                if (section === varp) sawSection = true;
                continue;
            }
            if (section !== varp || !line.startsWith('transmit=')) continue;
            const value = line.slice('transmit='.length).trim();
            if (value === 'yes') sawYes = true;
            else if (value === 'no') sawNo = true;
            else return 'unknown';
        }
    }
    if (!sawSection || (sawYes && sawNo)) return 'unknown';
    if (sawYes) return 'posted_by_content';
    return 'not_transmitted';
}

function lastLine(node: Node): number {
    if (node.kind === 'stmt') return node.line;
    const children = node.kind === 'if' ? [...node.then, ...node.els] : node.body;
    return children.reduce((last, child) => Math.max(last, lastLine(child)), node.line);
}

function hasDirectReturn(nodes: Node[]) {
    return nodes.some((node) => node.kind === 'stmt' && /^return\b/.test(node.text));
}

function guardedIncrementBound(
    nodes: Node[] | null,
    variable: string,
    writerLine: number,
    constants: Map<string, number | null>,
): number | null {
    if (!nodes) return null;
    let bound: number | null = null;
    const visit = (items: Node[]) => {
        for (const node of items) {
            if (node.kind === 'if') {
                if (
                    node.line < writerLine
                    && lastLine(node) < writerLine
                    && hasDirectReturn(node.then)
                    && !/[&]/.test(node.cond)
                ) {
                    const match = new RegExp(`%${variable}\\s*>=\\s*(?:\\^([A-Za-z0-9_]+)|(-?\\d+))`).exec(node.cond);
                    if (match) {
                        const value = match[1] ? constants.get(match[1]) : integer(match[2], `${variable} upper guard`);
                        if (typeof value === 'number') bound = bound === null ? value : Math.min(bound, value);
                    }
                }
                visit(node.then);
                visit(node.els);
            } else if (node.kind === 'while') {
                visit(node.body);
            }
        }
    };
    visit(nodes);
    return bound;
}

function stageDomain(
    variable: string,
    complete: number,
    constants: Map<string, number | null>,
    writersByVariable: Map<string, ProgressWriter[]>,
): ProgressMetadata['stage_domain'] {
    if (!variable) return null;
    let maximum = complete;
    let sawWriter = false;
    let safe = Number.isInteger(complete) && complete >= 0;
    // Quest-prefixed constants can be bit indices for a different varp.
    // Only this progress varp's actual writers may extend its completion bound.
    for (const writer of writersByVariable.get(variable) ?? []) {
        sawWriter = true;
        const literal = /^(-?\d+)$/.exec(writer.right);
        const constant = /^\^([A-Za-z0-9_]+)$/.exec(writer.right);
        if (literal || constant) {
            const value = literal
                ? integer(literal[1], `${writer.block.span.file}:${writer.line}`)
                : constants.get(constant![1]);
            if (typeof value !== 'number' || value < 0) safe = false;
            else maximum = Math.max(maximum, value);
            continue;
        }
        const increment = new RegExp(`^add\\(\\s*%${variable}\\s*,\\s*1\\s*\\)$`).test(writer.right);
        if (!increment) {
            safe = false;
            continue;
        }
        const bound = guardedIncrementBound(writer.nodes, variable, writer.line, constants);
        if (bound === null || bound < 0) safe = false;
        else maximum = Math.max(maximum, bound);
    }
    if (!sawWriter || !safe) return null;
    return { min: 0, max: maximum };
}

function colourHelperExists(questsText: string) {
    return parseRs2Blocks('scripts/general/scripts/quests.rs2', questsText)
        .filter((block) => block.kind === 'proc' && block.name === 'send_quest_progress_colour').length === 1;
}

function progressMetadata(
    content: string,
    quest: string,
    component: string,
    variable: string,
    complete: number,
    calls: { component: string; progress: string; complete: string }[],
    helperIsExact: boolean,
    sourceFiles: readonly string[],
    constants: Map<string, number | null>,
    writersByVariable: Map<string, ProgressWriter[]>,
): ProgressMetadata {
    const transmission = varpTransmission(content, variable, sourceFiles);
    const domain = stageDomain(variable, complete, constants, writersByVariable);
    const rows = calls.filter((call) => call.component === component);
    const direct = rows.length === 1
        && rows[0].progress === `%${variable}`
        && rows[0].complete === `^${quest}_complete`;
    return {
        colour_authoritative: direct
            && helperIsExact
            && transmission === 'not_transmitted'
            && domain !== null,
        stage_domain: domain,
        transmission,
    };
}

export function extractQuestIdentityFacts(content: string, revision: number) {
    const quests = requireGatherText(content, 'scripts/general/scripts/quests.rs2');
    const constants = parseQuestConstants(requireGatherText(content, 'scripts/general/configs/quest.constant'));
    const questlistPath = revision === 377 ? 'scripts/interfaces/questlist.if' : 'scripts/player/interfaces/questlist.if';
    const questlistText = requireGatherText(content, questlistPath);
    const listEntries = parseQuestListEntries(questlistText);
    const varpPack = parsePack(requireGatherText(content, 'pack/varp.pack'));
    if (varpPack.size === 0) throw new Error('pack/varp.pack: required file missing ids');
    const calls = parseQuestColourCalls(quests);
    const helperIsExact = colourHelperExists(quests);
    const journals = journalIndex(content);
    if (revision === 377) {
        const questConstants = questConstantsFor377(content, constants);
        const rows = questIdentityRowsFor377(
            content,
            listEntries,
            calls,
            questConstants.values,
            questConstants.ambiguous,
            varpPack,
            journals,
        );
        const barcrawl = barcrawlIdentityFor377(content, varpPack);
        if (!rows.some((row) => row.id === barcrawl.id)) rows.push(barcrawl);
        if (rows.length === 0) throw new Error('quest_identity: no extracted rows');
        if (rows.some((row) => row.requirements.qualification !== 'partial' || row.requirements.unknown_as_satisfied)) {
            throw new Error('quest_identity: requirements must stay partial');
        }
        return { rows, coverage: [] };
    }
    const sourceFiles = questIdentityAnalysisFiles(content);
    const progressConstantMap = progressConstants(content, sourceFiles);
    const progressSources = questProgressWriters(content, sourceFiles);
    const progressWriterIndex = progressSources.byVariable;
    const displays = parseQuestListText(questlistText);
    const seedIds = new Set(QUEST_IDENTITY_SEEDS.map((seed) => seed.id));
    const rows = QUEST_IDENTITY_SEEDS.map((seed) => {
        const found = calls.filter((call) => call.component === seed.component);
        if (found.length === 0) throw new Error(`quest_identity: no extracted rows; quests.rs2 has no colour call for ${seed.id}`);
        if (found.length !== 1) throw new Error(`quests.rs2: dual binding for ${seed.id}`);
        const call = found[0];
        const progress = /^%([A-Za-z0-9_]+)$/.exec(call.progress);
        if (!progress) {
            const why = call.progress.startsWith('~') ? 'proc operand' : 'not a %varp';
            throw new Error(`quests.rs2: ${seed.id} ${why} ${call.progress}`);
        }
        const varp = progress[1];
        const varpId = varpPack.get(varp);
        if (varpId === undefined) throw new Error(`quests.rs2: ${seed.id} colour operand %${varp} is absent from varp.pack`);
        const stem = `^${seed.id}_complete`;
        if (call.complete !== stem) {
            const why = call.complete.includes('(') || call.complete.includes(',') ? 'computed complete' : 'constant stem';
            throw new Error(`quests.rs2: ${seed.id} ${why} ${call.complete}`);
        }
        const complete = constants.get(`${seed.id}_complete`);
        const questPoints = constants.get(`${seed.id}_questpoints`);
        if (complete === undefined) throw new Error(`quest.constant: missing ^${seed.id}_complete`);
        if (questPoints === undefined) throw new Error(`quest.constant: missing ^${seed.id}_questpoints`);
        const metadata = progressMetadata(
            content,
            seed.id,
            seed.component,
            varp,
            complete,
            calls,
            helperIsExact,
            sourceFiles,
            progressConstantMap,
            progressWriterIndex,
        );
        const display = displays.get(seed.component);
        if (!display) throw new Error(`questlist.if: missing text= for ${seed.component}`);
        const list = listEntries.find((entry) => entry.component === seed.component);
        const journal = journals.get(seed.component);
        return {
            id: seed.id,
            component: seed.component,
            display,
            varp,
            varp_id: varpId,
            complete,
            quest_points: questPoints,
            colour_authoritative: metadata.colour_authoritative,
            stage_domain: metadata.stage_domain,
            transmission: metadata.transmission,
            unknown_sides: [] as string[],
            requirements: requirementsFor(seed.id, content),
            members: list?.members ?? false,
            kind: 'quest',
            journal_title: journal?.title ?? null,
            journal_script: journal?.script ?? null,
        };
    });
    let procIndex: Map<string, Rs2Block[]> | null = null;
    for (const entry of listEntries) {
        if (seedIds.has(entry.component)) continue;
        if (revision === 274 && entry.component === 'routequest') continue;
        const call = firstVarpCall(calls, entry.component);
        let varp = '';
        let varpId = -1;
        let complete = 0;
        const unresolved: string[] = [];
        if (call) {
            const progress = /^%([A-Za-z0-9_]+)$/.exec(call.progress);
            if (progress) {
                varp = progress[1];
                varpId = varpPack.get(varp) ?? -1;
            } else if (call.progress.startsWith('~')) {
                const resolved = resolveProgressProc((procIndex ??= procDefinitions(content)), call.progress);
                const resolvedId = resolved === null ? undefined : varpPack.get(resolved);
                if (resolved !== null && resolvedId !== undefined) {
                    varp = resolved;
                    varpId = resolvedId;
                } else {
                    varp = call.progress;
                    unresolved.push(`progress operand ${call.progress} does not resolve to one varp in pack/varp.pack`);
                }
            } else {
                varp = call.progress;
                unresolved.push(`progress operand ${call.progress} is not a %varp or a proc`);
            }
            if (call.complete.startsWith('^')) {
                const name = call.complete.slice(1);
                const known = constants.get(name);
                const local = known === undefined ? questLocalConstant(content, entry.component, name) : null;
                if (known !== undefined) complete = known;
                else if (local !== null) complete = local;
                else unresolved.push(`complete constant ${call.complete} is not in quest.constant or the quest's own .constant`);
            } else {
                unresolved.push(`complete operand ${call.complete} is not a constant`);
            }
        } else {
            unresolved.push('no colour call; the varp stays empty');
        }
        if (unresolved.length > 0) {
            console.warn(`quest_identity: ${entry.component} unresolved: ${unresolved.join('; ')}; kept today's row values`);
        }
        const questPoints = entry.component === 'fortress'
            ? (constants.get('blackknight_questpoints') ?? 0)
            : (constants.get(`${entry.component}_questpoints`) ?? 0);
        const metadata = progressMetadata(
            content,
            entry.component,
            entry.component,
            varp,
            complete,
            calls,
            helperIsExact,
            sourceFiles,
            progressConstantMap,
            progressWriterIndex,
        );
        const journal = journals.get(entry.component);
        rows.push({
            id: entry.component,
            component: entry.component,
            display: entry.display,
            varp,
            varp_id: varpId,
            complete,
            quest_points: questPoints,
            colour_authoritative: metadata.colour_authoritative,
            stage_domain: metadata.stage_domain,
            transmission: metadata.transmission,
            unknown_sides: [],
            requirements: emptyMustHave(),
            members: entry.members,
            kind: entry.stub ? 'stub' : 'quest',
            journal_title: journal?.title ?? null,
            journal_script: journal?.script ?? null,
        });
    }
    if (listEntries.length >= 60 && !rows.some((row) => row.id === 'barcrawl')) {
        rows.push({
            id: 'barcrawl',
            component: 'barcrawl',
            display: "Alfred Grimhand's Barcrawl",
            varp: 'barcrawl',
            varp_id: varpPack.get('barcrawl') ?? 77,
            complete: 2,
            quest_points: 0,
            colour_authoritative: false,
            stage_domain: null,
            transmission: 'unknown',
            unknown_sides: [],
            requirements: emptyMustHave(),
            members: true,
            kind: 'miniquest',
            journal_title: null,
            journal_script: null,
        });
    }
    if (rows.length === 0) throw new Error('quest_identity: no extracted rows');
    const postWriterConstants = new Map<string, number>();
    for (const [name, value] of progressConstantMap) {
        if (value !== null) postWriterConstants.set(name, value);
    }
    const checked = new Set<string>();
    const postCompletionVariables = new Set<string>();
    for (const row of rows) {
        if (row.varp_id < 0 || row.complete <= 0 || postCompletionVariables.has(row.varp)) continue;
        const key = `${row.varp}:${row.complete}`;
        if (checked.has(key)) continue;
        checked.add(key);
        const writers = progressWriterIndex.get(row.varp) ?? [];
        if (writers.length > 0 && postCompletionProgressWriters(
            progressSources.blocks,
            row.varp,
            { min: row.complete, max: row.stage_domain?.max ?? 2147483647 },
            postWriterConstants,
            writers,
        ).length > 0) postCompletionVariables.add(row.varp);
    }
    for (const row of rows) {
        if (postCompletionVariables.has(row.varp)) row.colour_authoritative = false;
    }
    if (rows.some((row) => row.requirements.qualification !== 'partial' || row.requirements.unknown_as_satisfied)) {
        throw new Error('quest_identity: requirements must stay partial');
    }
    const coverage: {
        class: 'revision-absent';
        alias: string;
        on_revision: number;
        copied: false;
        reason: string;
    }[] = [];
    if (revision === 274 && !varpPack.has('routequest')) {
        coverage.push({
            class: 'revision-absent',
            alias: 'routequest',
            on_revision: 274,
            copied: false,
            reason: '289-only quest, not copied onto 274',
        });
    }
    const facts = { rows, coverage };
    if (JSON.stringify(facts).includes('family-unavailable')) throw new Error('quest_identity: must not emit family-unavailable');
    return facts;
}
