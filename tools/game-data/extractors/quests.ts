// M-296 owns full-roster extraction/resolution programs; this preserves the existing query asset.
import { integer, parsePack, requireGatherText } from './common.ts';
export const questIdentityContentFiles = [
    'scripts/general/scripts/quests.rs2',
    'scripts/general/configs/quest.constant',
    'scripts/player/interfaces/questlist.if',
    'scripts/quests/quest_cook/scripts/quest_cook.rs2',
    'scripts/quests/quest_waterfall/scripts/quest_waterfall.rs2',
    'scripts/quests/quest_zanaris/scripts/quest_zanaris.rs2',
];
const QUEST_IDENTITY_SEEDS = [
    { id: 'cook', component: 'cook' },
    { id: 'runemysteries', component: 'runemysteries' },
    { id: 'murder', component: 'murder' },
    { id: 'waterfall', component: 'waterfall' },
    { id: 'death', component: 'death' },
    { id: 'zanaris', component: 'zanaris' },
] as const;

type QuestItemAlias = { alias: string; quantity: number | null; kind: 'inv' | 'use-site' };
type QuestSkillGate = { skill: string; level: number };
type QuestRequirements = {
    qualification: 'partial';
    skills: QuestSkillGate[];
    items: QuestItemAlias[];
    empty_must_have: boolean;
    unknown_as_satisfied: false;
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

function parseQuestConstants(text: string) {
    const out = new Map<string, number>();
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        const match = /^\^([A-Za-z0-9_]+)\s*=\s*(-?\d+)\s*(?:\/\/.*)?$/.exec(line);
        if (!match) continue;
        if (out.has(match[1])) throw new Error(`quest.constant: duplicate ^${match[1]}`);
        out.set(match[1], integer(match[2], `^${match[1]}`));
    }
    if (out.size === 0) throw new Error('quest.constant: no constants');
    return out;
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

function parseQuestEnumDisplays(text: string) {
    const names = new Set<string>();
    let inQuestNames = false;
    let saw = false;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            const section = line.slice(1, -1);
            if (inQuestNames && section !== 'quest_names_enum') break;
            inQuestNames = section === 'quest_names_enum';
            if (inQuestNames) saw = true;
            continue;
        }
        if (!inQuestNames || !line.startsWith('val=')) continue;
        const comma = line.indexOf(',');
        if (comma < 0) throw new Error(`quest.enum: malformed val line ${line}`);
        const name = line.slice(comma + 1).trim();
        if (!name) throw new Error(`quest.enum: empty display ${line}`);
        if (names.has(name)) throw new Error(`quest.enum: duplicate display ${name}`);
        names.add(name);
    }
    if (!saw || names.size === 0) throw new Error('quest.enum: missing [quest_names_enum]');
    return names;
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
    if (id === 'runemysteries' || id === 'murder' || id === 'death') return emptyMustHave();
    throw new Error(`quest_identity: unknown seed ${id}`);
}

export function extractQuestIdentityFacts(content: string, revision: number) {
    const quests = requireGatherText(content, 'scripts/general/scripts/quests.rs2');
    const constants = parseQuestConstants(requireGatherText(content, 'scripts/general/configs/quest.constant'));
    const displays = parseQuestListText(requireGatherText(content, 'scripts/player/interfaces/questlist.if'));
    const varpPack = parsePack(requireGatherText(content, 'pack/varp.pack'));
    const enumNames = parseQuestEnumDisplays(requireGatherText(content, 'scripts/general/configs/quest.enum'));
    if (varpPack.size === 0) throw new Error('pack/varp.pack: required file missing ids');
    const calls = parseQuestColourCalls(quests);
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
        const display = displays.get(seed.component);
        if (!display) throw new Error(`questlist.if: missing text= for ${seed.component}`);
        if (!enumNames.has(display)) throw new Error(`quest.enum: display mismatch for ${seed.id}: ${display}`);
        return {
            id: seed.id,
            component: seed.component,
            display,
            varp,
            varp_id: varpId,
            complete,
            quest_points: questPoints,
            unknown_sides: [] as string[],
            requirements: requirementsFor(seed.id, content),
        };
    });
    if (rows.length !== QUEST_IDENTITY_SEEDS.length) throw new Error('quest_identity: no extracted rows');
    if (rows.some((row) => row.requirements.qualification !== 'partial' || row.requirements.unknown_as_satisfied)) {
        throw new Error('quest_identity: requirements must stay partial');
    }
    const coverage: {
        class: 'revision-absent';
        alias: string;
        on_revision: number;
        other_pin_id: number;
        copied: false;
        reason: string;
    }[] = [];
    if (revision === 274) {
        if (varpPack.has('routequest')) throw new Error('274: routequest is in varp.pack; do not copy it and do not emit revision-absent');
        coverage.push({
            class: 'revision-absent',
            alias: 'routequest',
            on_revision: 274,
            other_pin_id: 387,
            copied: false,
            reason: '289-only quest, not copied onto 274',
        });
    }
    const facts = { rows, coverage };
    if (JSON.stringify(facts).includes('family-unavailable')) throw new Error('quest_identity: must not emit family-unavailable');
    return facts;
}
