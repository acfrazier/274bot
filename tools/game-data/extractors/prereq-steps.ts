import fs from 'node:fs';
import path from 'node:path';
import { integer, parseJm2LocPlacements, parseJm2NpcPlacements, parseMapsquarePath, parsePack, sourceFile, walkContentFiles, worldFromMapsquare } from './common.ts';
import { assertPrereqPreservation } from './prereq-preservation.ts';
import type { InputHash } from '../generate.ts';

export const PREREQ_STEPS_SCHEMA = 1;
export type RangeWire = { min: number; max: number };
export type TileWire = { x: number; z: number; level: number };
export type SourceWire = string;
export type CrossingWire = {
    crossing: { loc_id: number; option: number; at: TileWire; to: TileWire; kind: string };
    pass: { message: string; landing: TileWire; proves: RangeWire; source: SourceWire };
    delay: { ticks: number; source?: SourceWire };
    source: SourceWire;
};
export type PrereqRowWire = {
    id: string;
    crossings: CrossingWire[];
    establishes: { quest: string; signal: string; values: RangeWire };
    after: { proves: RangeWire; after: RangeWire }[];
    reviewed_writes: { write: string; sets: number; guard: string; guard_text: string; called_from: string }[];
    probe: { refuse: { page: string; then: string | { precondition: unknown }; proves: RangeWire; source: SourceWire }[] };
    target: { npc: number; op: number; anchor: TileWire; leash: number; source: SourceWire };
    preconditions: unknown[];
    talk: {
        strict: boolean;
        prefer: string[];
        line_rules: { page: string; choice: string; source: SourceWire }[];
        branches: {
            opener: string;
            outcome: string;
            effect_page?: string;
            terminal?: string;
            proves: RangeWire;
            source: SourceWire;
            requires_at_opener?: { kind: string; count: number; otherwise: string };
            exit?: string;
        }[];
        repeat_after?: { reclaimed?: number };
    };
    memory: string;
    consent: string;
};
export type PrereqStepFacts = { rows: PrereqRowWire[] };

const AUTHORED_FILE = path.resolve(import.meta.dirname, '../prereq-steps.json');
const AUTHORED = JSON.parse(fs.readFileSync(AUTHORED_FILE, 'utf8')) as PrereqStepFacts;

function parseSource(source: string) {
    return source.split(';').map((part) => {
        const match = /^\s*(.+?):(\d+(?:-\d+)?(?:,\d+(?:-\d+)?)*)\s*$/.exec(part);
        if (!match) throw new Error(`prereq_steps: invalid source citation ${part}`);
        const lines = match[2].split(',').flatMap((selection) => {
            const range = /^(\d+)(?:-(\d+))?$/.exec(selection)!;
            const first = integer(range[1], `${match[1]} line`);
            const last = integer(range[2] ?? range[1], `${match[1]} line`);
            if (first < 1 || last < first) throw new Error(`prereq_steps: invalid line range ${part}`);
            return Array.from({ length: last - first + 1 }, (_, index) => first + index);
        });
        return { file: match[1], lines };
    });
}

function citedText(content: string, source: string) {
    return parseSource(source).map(({ file, lines }) => {
        const all = fs.readFileSync(path.join(content, file), 'utf8').split(/\r?\n/);
        for (const line of lines) if (line > all.length) throw new Error(`prereq_steps: ${file}:${line} is outside the source file`);
        return lines.map((line) => all[line - 1]).join('\n');
    }).join('\n');
}

function isProductionScript(file: string) {
    return !file.split(path.sep).some((segment) => segment === '_test' || segment === 'debug');
}
function sourcePaths(content: string, rows: PrereqRowWire[]) {
    const found = new Set<string>();
    const add = (source: string) => {
        for (const citation of parseSource(source)) found.add(citation.file);
    };
    for (const extension of ['.rs2', '.constant', '.varp']) {
        for (const file of walkContentFiles(path.join(content, 'scripts'), extension)) {
            if (!isProductionScript(file)) continue;
            found.add(path.relative(content, file).split(path.sep).join('/'));
        }
    }
    for (const file of ['pack/loc.pack', 'pack/npc.pack', 'pack/varp.pack', 'scripts/general/configs/quest.enum', 'scripts/player/interfaces/questlist.if']) {
        found.add(file);
    }
    for (const row of rows) {
        for (const crossing of row.crossings) {
            add(crossing.source);
            add(crossing.pass.source);
            if (crossing.delay.source) add(crossing.delay.source);
        }
        for (const refusal of row.probe.refuse) add(refusal.source);
        add(row.target.source);
        for (const precondition of row.preconditions) {
            if (typeof precondition === 'object' && precondition !== null && 'source' in precondition && typeof precondition.source === 'string') add(precondition.source);
        }
        for (const write of row.reviewed_writes) {
            add(write.write);
            add(write.guard);
            add(write.called_from);
        }
        for (const branch of row.talk.branches) add(branch.source);
        for (const rule of row.talk.line_rules) add(rule.source);
    }
    return [...found].sort();
}

function textPins(content: string, rows: PrereqRowWire[]) {
    const pin = (source: string, value: string, label: string) => {
        if (!citedText(content, source).includes(value)) throw new Error(`prereq_steps: ${label} is not pinned by ${source}`);
    };
    for (const row of rows) {
        for (const crossing of row.crossings) pin(crossing.pass.source, crossing.pass.message, `${row.id} pass message`);
        for (const refusal of row.probe.refuse) pin(refusal.source, refusal.page, `${row.id} refusal page`);
        for (const write of row.reviewed_writes) pin(write.guard, write.guard_text, `${row.id} reviewed guard`);
        for (const branch of row.talk.branches) {
            if (branch.opener !== '*') pin(branch.source, branch.opener, `${row.id} opener`);
            if (branch.effect_page) pin(branch.source, branch.effect_page, `${row.id} effect page`);
            if (branch.terminal) pin(branch.source, branch.terminal, `${row.id} terminal`);
        }
        for (const preferred of row.talk.prefer) {
            if (!row.talk.branches.some((branch) => citedText(content, branch.source).includes(preferred))) {
                throw new Error(`prereq_steps: ${row.id} preferred dialogue line is not pinned`);
            }
        }
        for (const rule of row.talk.line_rules) {
            pin(rule.source, rule.page, `${row.id} line-rule page`);
            pin(rule.source, rule.choice, `${row.id} line-rule choice`);
        }
    }
}

type CheckedPrecondition = { kind?: unknown; source?: unknown; gate?: unknown; item?: unknown; otherwise?: unknown };
type CheckedGate = { Complete?: unknown; Window?: unknown };
type CheckedWindow = { quest?: unknown; signal?: unknown; values?: unknown };
type CheckedRange = { min?: unknown; max?: unknown };
type CheckedOwnedItem = { item?: unknown };

function scriptConstants(content: string) {
    const values = new Map<string, number>();
    for (const file of walkContentFiles(path.join(content, 'scripts'), '.constant').filter(isProductionScript)) {
        const relative = path.relative(content, file);
        for (const raw of fs.readFileSync(file, 'utf8').split(/\r?\n/)) {
            const match = /^\s*\^([A-Za-z0-9_]+)\s*=\s*(-?\d+)\s*(?:\/\/.*)?$/.exec(raw);
            if (!match) continue;
            const value = integer(match[2], `${relative}: ^${match[1]}`);
            const previous = values.get(match[1]);
            if (previous !== undefined && previous !== value) {
                throw new Error(`prereq_steps: conflicting ^${match[1]} constants (${previous}, ${value})`);
            }
            values.set(match[1], value);
        }
    }
    return values;
}

function escapeRegExp(value: string) {
    return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function verifyPreconditions(content: string, rows: PrereqRowWire[], constants: ReadonlyMap<string, number>) {
    for (const row of rows) for (const value of row.preconditions) {
        if (typeof value !== 'object' || value === null || Array.isArray(value)) {
            throw new Error(`prereq_steps: ${row.id} has a malformed precondition`);
        }
        const precondition = value as CheckedPrecondition;
        if (typeof precondition.kind !== 'string' || typeof precondition.source !== 'string') {
            throw new Error(`prereq_steps: ${row.id} has a malformed precondition`);
        }
        const source = citedText(content, precondition.source);
        if (precondition.kind === 'gate') {
            if (typeof precondition.gate !== 'object' || precondition.gate === null || Array.isArray(precondition.gate)) {
                throw new Error(`prereq_steps: ${row.id} gate precondition is malformed`);
            }
            const gate = precondition.gate as CheckedGate;
            if (typeof gate.Complete === 'string') {
                const quest = gate.Complete;
                if (!new RegExp(`%${escapeRegExp(quest)}\\s*=\\s*\\^${escapeRegExp(quest)}_complete\\b`).test(source)) {
                    throw new Error(`prereq_steps: ${row.id} Complete(${quest}) is not pinned by ${precondition.source}`);
                }
                continue;
            }
            if (typeof gate.Window !== 'object' || gate.Window === null || Array.isArray(gate.Window)) {
                throw new Error(`prereq_steps: ${row.id} window precondition is malformed`);
            }
            const window = gate.Window as CheckedWindow;
            if (typeof window.quest !== 'string' || typeof window.signal !== 'string'
                || typeof window.values !== 'object' || window.values === null || Array.isArray(window.values)) {
                throw new Error(`prereq_steps: ${row.id} window precondition is malformed`);
            }
            const range = window.values as CheckedRange;
            if (typeof range.min !== 'number' || range.min !== range.max) {
                throw new Error(`prereq_steps: ${row.id} window precondition is not an exact stage`);
            }
            const match = new RegExp(`%${escapeRegExp(window.signal)}\\s*=\\s*\\^([A-Za-z0-9_]+)\\b`).exec(source);
            if (!match || !match[1].startsWith(`${window.quest}_`) || constants.get(match[1]) !== range.min) {
                throw new Error(`prereq_steps: ${row.id} ${window.signal} window is not pinned by ${precondition.source}`);
            }
        } else if (precondition.kind === 'owned_anywhere') {
            if (typeof precondition.item !== 'string') throw new Error(`prereq_steps: ${row.id} owned-item precondition is malformed`);
            for (const holder of ['inv', 'bank', 'worn']) {
                if (!source.includes(`inv_total(${holder}, ${precondition.item})`)) {
                    throw new Error(`prereq_steps: ${row.id} ${precondition.item} is not checked in ${holder}`);
                }
            }
            if (precondition.otherwise !== 'reclaim') {
                throw new Error(`prereq_steps: ${row.id} has an unsupported owned-item fallback`);
            }
            if (!/@reclaim_[A-Za-z0-9_]+;?/.test(source)) {
                throw new Error(`prereq_steps: ${row.id} missing reclaim branch in ${precondition.source}`);
            }
        } else {
            throw new Error(`prereq_steps: ${row.id} has an unsupported precondition kind ${precondition.kind}`);
        }
    }
}

function verifyTalkEffects(content: string, rows: PrereqRowWire[], constants: ReadonlyMap<string, number>) {
    for (const row of rows) {
        const stage = row.establishes.values;
        const stageBranches = row.talk.branches.filter((branch) => branch.outcome === 'advice' || branch.outcome === 'start');
        if (stage.min !== stage.max || stageBranches.length === 0) {
            throw new Error(`prereq_steps: ${row.id} has no exact authored stage-setting branch`);
        }
        for (const branch of stageBranches) {
            const source = citedText(content, branch.source);
            const assignment = new RegExp(`%${escapeRegExp(row.establishes.signal)}\\s*=\\s*\\^([A-Za-z0-9_]+)\\s*;`);
            const constant = assignment.exec(source)?.[1];
            if (!constant || constants.get(constant) !== stage.min) {
                throw new Error(`prereq_steps: ${row.id} stage-setting branch is not pinned by ${branch.source}`);
            }
        }
        const ownedValue = row.preconditions.find((value) => typeof value === 'object' && value !== null
            && !Array.isArray(value) && 'kind' in value && value.kind === 'owned_anywhere');
        if (!ownedValue) continue;
        const owned = ownedValue as CheckedOwnedItem;
        if (typeof owned.item !== 'string') continue;
        for (const branch of row.talk.branches.filter((entry) => entry.outcome === 'reclaimed')) {
            if (!citedText(content, branch.source).includes(`inv_add(inv, ${owned.item}, 1)`)) {
                throw new Error(`prereq_steps: ${row.id} reclaim branch does not grant ${owned.item}`);
            }
        }
    }
}

function verifyPlacements(content: string, rows: PrereqRowWire[]) {
    const locIds = new Set(parsePack(fs.readFileSync(path.join(content, 'pack/loc.pack'), 'utf8')).values());
    const npcIds = new Set(parsePack(fs.readFileSync(path.join(content, 'pack/npc.pack'), 'utf8')).values());
    for (const row of rows) {
        if (!npcIds.has(row.target.npc)) throw new Error(`prereq_steps: ${row.id} NPC ${row.target.npc} is absent from npc.pack`);
        for (const crossing of row.crossings) {
            if (!locIds.has(crossing.crossing.loc_id)) throw new Error(`prereq_steps: ${row.id} loc ${crossing.crossing.loc_id} is absent from loc.pack`);
            if (JSON.stringify(crossing.pass.landing) !== JSON.stringify(crossing.crossing.to)) {
                throw new Error(`prereq_steps: ${row.id} crossing destination differs from pass landing`);
            }
            const map = parseSource(crossing.source).find((citation) => citation.file.endsWith('.jm2'));
            if (!map) throw new Error(`prereq_steps: ${row.id} crossing has no map placement citation`);
            const mapText = fs.readFileSync(path.join(content, map.file), 'utf8').split(/\r?\n/);
            const { mx, mz } = parseMapsquarePath(map.file);
            const placements = map.lines.flatMap((line) => {
                const placement = parseJm2LocPlacements(`==== LOC ====\n${mapText[line - 1]}`, new Set([crossing.crossing.loc_id]))[0];
                return placement ? [worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane)] : [];
            });
            if (placements.length !== 1 || placements[0].x !== crossing.crossing.at.x || placements[0].z !== crossing.crossing.at.z || placements[0].plane !== crossing.crossing.at.level) {
                throw new Error(`prereq_steps: ${row.id} crossing ${crossing.crossing.loc_id} does not match its cited map tile`);
            }
            const jumps = [...citedText(content, crossing.pass.source).matchAll(/\bp_telejump\((\d+)_(\d+)_(\d+)_(\d+)_(\d+)\)/g)];
            if (jumps.length > 0) {
                const landsAtSource = jumps.some((match) => {
                    const tile = worldFromMapsquare(
                        integer(match[2], 'teleport mapsquare x'),
                        integer(match[3], 'teleport mapsquare z'),
                        integer(match[4], 'teleport local x'),
                        integer(match[5], 'teleport local z'),
                        integer(match[1], 'teleport level'),
                    );
                    return tile.x === crossing.pass.landing.x && tile.z === crossing.pass.landing.z && tile.plane === crossing.pass.landing.level;
                });
                if (!landsAtSource) throw new Error(`prereq_steps: ${row.id} pass landing differs from its p_telejump source`);
            } else if (crossing.crossing.kind === 'door') {
                const distance = Math.abs(crossing.crossing.at.x - crossing.crossing.to.x) + Math.abs(crossing.crossing.at.z - crossing.crossing.to.z);
                if (distance !== 1 || crossing.crossing.at.level !== crossing.crossing.to.level) {
                    throw new Error(`prereq_steps: ${row.id} door landing is not the adjacent tile`);
                }
            }
            const actualDelay = crossing.delay.source
                ? parseSource(crossing.delay.source).reduce((sum, citation) => {
                    const lines = fs.readFileSync(path.join(content, citation.file), 'utf8').split(/\r?\n/);
                    return sum + citation.lines.reduce((subsum, line) => {
                        const delays = [...lines[line - 1].matchAll(/\bp_delay\((\d+)\)/g)];
                        if (delays.length === 0) throw new Error(`prereq_steps: ${citation.file}:${line} contains no p_delay`);
                        return subsum + delays.reduce((delaySum, match) => delaySum + integer(match[1], 'p_delay') + 1, 0);
                    }, 0);
                }, 0)
                : 0;
            if (actualDelay !== crossing.delay.ticks) throw new Error(`prereq_steps: ${row.id} crossing delay ${crossing.delay.ticks} differs from pinned p_delay count ${actualDelay}`);
        }
        const targetMap = parseSource(row.target.source).find((citation) => citation.file.endsWith('.jm2'));
        if (!targetMap) throw new Error(`prereq_steps: ${row.id} target has no map citation`);
        const targetText = fs.readFileSync(path.join(content, targetMap.file), 'utf8').split(/\r?\n/);
        const { mx, mz } = parseMapsquarePath(targetMap.file);
        const npcs = targetMap.lines.flatMap((line) => {
            const placement = parseJm2NpcPlacements(`==== NPC ====\n${targetText[line - 1]}`).find((entry) => entry.npc_id === row.target.npc);
            return placement ? [worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane)] : [];
        });
        if (npcs.length !== 1 || npcs[0].x !== row.target.anchor.x || npcs[0].z !== row.target.anchor.z || npcs[0].plane !== row.target.anchor.level) {
            throw new Error(`prereq_steps: ${row.id} NPC target does not match its cited map tile`);
        }
    }
}

export function extractPrereqStepsFacts(revision: number): PrereqStepFacts {
    return revision === 289 ? AUTHORED : { rows: [] };
}

export function assertPrereqStepFacts(
    content: string,
    facts: PrereqStepFacts,
    questRows?: readonly { id: string; colour_authoritative: boolean; transmission: string; stage_domain: RangeWire | null }[],
) {
    if (facts.rows.length !== 2 || facts.rows.map((row) => row.id).join('|') !== 'priestperil:drezel-advice|druidspirit:drezel-start') {
        throw new Error('prereq_steps: expected the reviewed A1/A2 rows in route order');
    }
    textPins(content, facts.rows);
    const constants = scriptConstants(content);
    verifyPreconditions(content, facts.rows, constants);
    verifyTalkEffects(content, facts.rows, constants);
    verifyPlacements(content, facts.rows);
    assertPrereqPreservation(content, facts.rows, questRows);
    for (const row of facts.rows) {
        if (row.crossings.some((crossing) => crossing.delay.ticks < 0 || crossing.delay.ticks > 65535)) {
            throw new Error(`prereq_steps: invalid crossing delay in ${row.id}`);
        }
    }
}

export function prereqStepInputs(content: string, facts: PrereqStepFacts): InputHash[] {
    return sourcePaths(content, facts.rows).map((file) => ({ ...sourceFile(content, file), path: `content/${file}` }));
}
