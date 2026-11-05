import fs from 'node:fs';
import path from 'node:path';
import { integer, parseJm2LocPlacements, parseJm2NpcPlacements, parseMapsquarePath, parsePack, walkContentFiles, worldFromMapsquare } from './common.ts';
import { assertPrereqPreservation } from './prereq-preservation.ts';
import { parseRs2Blocks, stripComment, stripComments } from './gathering-content.ts';
import { parseBody } from './gathering-rs2.ts';
import { captureProgressWriterDomains, type ProgressWriterDomains } from './quests.ts';
import { boundedJournalRange } from './prereq-journal-proof.ts';

export const PREREQ_STEPS_SCHEMA = 3;
export type RangeWire = { min: number; max: number };
export type TileWire = { x: number; z: number; level: number };
export type SourceWire = string;
export type CrossingWire = {
    crossing: { loc_id: number; option: number; at: TileWire; takeoff: TileWire; to: TileWire; kind: string };
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
export type PrereqJournalRuleWire = { all: string[]; proves: RangeWire; source: SourceWire };
export type PrereqJournalWire = {
    id: string;
    quest: string;
    establishes: { quest: string; signal: string; values: RangeWire };
    after: { proves: RangeWire; after: RangeWire }[];
    reviewed_writes: PrereqRowWire['reviewed_writes'];
    rules: PrereqJournalRuleWire[];
    source: SourceWire;
};
export type PrereqStepFacts = { rows: PrereqRowWire[]; journals: PrereqJournalWire[] };

const AUTHORED_FILE = path.resolve(import.meta.dirname, '../prereq-steps.json');

/**
 * Authored crossing template: source-pinned content only. The template keeps
 * the loc/option/kind selector plus the pass/delay/source citations, but must
 * never carry at/to/takeoff/landing coordinates: every generated coordinate
 * derives from the CLI shared pack projection (see `loadPrereqCrossingProjection`).
 * `heading` selects one directed door heading from the actual takeoff->to
 * geometry (`south` = the pack step lands one south of its takeoff); omitting
 * it expands to every distinct legal takeoff (the A1 barrier).
 */
export type CrossingTemplate = {
    loc_id: number;
    option: number;
    kind: string;
    heading?: 'north' | 'south' | 'east' | 'west';
    pass: { message: string; proves: RangeWire; source: SourceWire };
    delay: { ticks: number; source?: SourceWire };
    source: SourceWire;
};
export type AuthoredPrereqRow = Omit<PrereqRowWire, 'crossings'> & { crossings: CrossingTemplate[] };
export type AuthoredPrereqTemplates = { rows: AuthoredPrereqRow[]; journals: PrereqJournalWire[] };

function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}
function isRangeWire(value: unknown): value is RangeWire {
    return isRecord(value) && Number.isInteger(value.min) && Number.isInteger(value.max)
        && (value.min as number) <= (value.max as number);
}

function equalRangeWire(left: RangeWire, right: RangeWire) {
    return left.min === right.min && left.max === right.max;
}

function loadAuthoredTemplates(): AuthoredPrereqTemplates {
    const parsed: unknown = JSON.parse(fs.readFileSync(AUTHORED_FILE, 'utf8'));
    if (!isRecord(parsed) || !Array.isArray(parsed.rows) || !Array.isArray(parsed.journals)
        || Object.keys(parsed).join(',') !== 'rows,journals') {
        throw new Error('prereq_steps: authored templates must be exactly {rows,journals}');
    }
    for (const row of parsed.rows) {
        if (!isRecord(row) || !Array.isArray(row.crossings)) throw new Error('prereq_steps: authored row is malformed');
        for (const template of row.crossings) {
            if (!isRecord(template)) throw new Error(`prereq_steps: ${(row.id as string) ?? '?'} has a malformed crossing template`);
            for (const banned of ['at', 'to', 'takeoff', 'landing']) {
                if (banned in template || (isRecord(template.pass) && banned === 'landing' && 'landing' in template.pass)) {
                    throw new Error(`prereq_steps: authored crossing coordinates are forbidden (${banned}); they derive from the pack projection`);
                }
            }
            const keys = Object.keys(template).sort().join(',');
            if (keys !== 'delay,kind,loc_id,option,pass,source' && keys !== 'delay,heading,kind,loc_id,option,pass,source') {
                throw new Error(`prereq_steps: ${(row.id as string) ?? '?'} crossing template fields are malformed (${keys})`);
            }
            if (!Number.isInteger(template.loc_id) || !Number.isInteger(template.option)
                || (template.option as number) < 0 || (template.option as number) > 255) {
                throw new Error(`prereq_steps: ${(row.id as string) ?? '?'} crossing template has an invalid loc/option selector`);
            }
            const templateKind: unknown = template.kind;
            if (typeof templateKind !== 'string' || !KNOWN_CROSSING_KINDS.has(templateKind)) {
                throw new Error(`prereq_steps: ${(row.id as string) ?? '?'} crossing template has an unknown kind`);
            }
            const templateHeading: unknown = template.heading;
            if (templateHeading !== undefined && (typeof templateHeading !== 'string' || !KNOWN_HEADINGS.has(templateHeading))) {
                throw new Error(`prereq_steps: ${(row.id as string) ?? '?'} crossing template has an unknown heading`);
            }
            if (!isRecord(template.pass) || typeof template.pass.message !== 'string' || template.pass.message.length === 0
                || !isRecord(template.pass.proves) || typeof template.pass.source !== 'string'
                || Object.keys(template.pass).sort().join(',') !== 'message,proves,source') {
                throw new Error(`prereq_steps: ${(row.id as string) ?? '?'} crossing template pass is malformed`);
            }
            if (!isRecord(template.delay) || !Number.isInteger(template.delay.ticks)
                || (template.delay.ticks as number) < 0 || (template.delay.ticks as number) > 65535
                || (template.delay.source !== undefined && typeof template.delay.source !== 'string')) {
                throw new Error(`prereq_steps: ${(row.id as string) ?? '?'} crossing template delay is malformed`);
            }
            if (typeof template.source !== 'string' || template.source.length === 0) {
                throw new Error(`prereq_steps: ${(row.id as string) ?? '?'} crossing template source is malformed`);
            }
        }
    }
    for (const value of parsed.journals) {
        if (!isRecord(value) || Object.keys(value).sort().join(',') !== 'after,establishes,id,quest,reviewed_writes,rules,source'
            || typeof value.id !== 'string' || value.id.length === 0
            || typeof value.quest !== 'string' || value.quest.length === 0
            || typeof value.source !== 'string' || value.source.length === 0
            || !isRecord(value.establishes) || Object.keys(value.establishes).sort().join(',') !== 'quest,signal,values'
            || typeof value.establishes.quest !== 'string' || value.establishes.quest.length === 0
            || typeof value.establishes.signal !== 'string' || value.establishes.signal.length === 0
            || !isRangeWire(value.establishes.values)
            || !Array.isArray(value.after) || !Array.isArray(value.reviewed_writes) || !Array.isArray(value.rules) || value.rules.length === 0) {
            throw new Error('prereq_steps: authored journal is malformed');
        }
        for (const rule of value.after) {
            if (!isRecord(rule) || Object.keys(rule).sort().join(',') !== 'after,proves'
                || !isRangeWire(rule.proves) || !isRangeWire(rule.after)) {
                throw new Error(`prereq_steps: ${value.id} journal after rule is malformed`);
            }
        }
        for (const rule of value.rules) {
            if (!isRecord(rule) || Object.keys(rule).sort().join(',') !== 'all,proves,source'
                || !Array.isArray(rule.all) || rule.all.length === 0
                || !rule.all.every((needle) => typeof needle === 'string' && needle.trim().length > 0)
                || !isRangeWire(rule.proves) || typeof rule.source !== 'string' || rule.source.length === 0) {
                throw new Error(`prereq_steps: ${value.id} journal text rule is malformed`);
            }
        }
        const journal = value as unknown as PrereqJournalWire;
        for (const rule of journal.rules) {
            if (journal.after.filter((after) => equalRangeWire(after.proves, rule.proves)).length !== 1) {
                throw new Error(`prereq_steps: ${journal.id} needs exactly one after rule for each journal proof`);
            }
        }
    }
    return parsed as unknown as AuthoredPrereqTemplates;
}

// ---------------------------------------------------------------------------
// CLI shared pack projection (owned by PackCrossingProjection; consumed here).
// ---------------------------------------------------------------------------

/** CLI crossing projection schema; the family wire has its own schema. */
export const PREREQ_CROSSING_PROJECTION_SCHEMA = 2;
/** Lower-case CrossingKind serde representation, matching the Rust decoder. */
const KNOWN_CROSSING_KINDS = new Set([
    'door', 'ladder', 'stairs', 'boat', 'teleport',
    'agility_shortcut', 'glider', 'spirit_tree', 'npc', 'essence_exit',
]);
const KNOWN_HEADINGS = new Set(['north', 'south', 'east', 'west']);

export type ProjectionTile = { x: number; z: number; level: number };
export type ProjectionCrossing = {
    loc_id: number;
    option: number;
    at: ProjectionTile;
    takeoff: ProjectionTile;
    to: ProjectionTile;
    kind: string;
};
export type ProjectionEntry = {
    crossing: ProjectionCrossing;
    standable: boolean;
    admissible: boolean;
    /** Disjunction of conjunctive quest-gate arrays: gates within one inner
     * array are conjunctive; the outer arrays are alternative encoded edge
     * conditions for this same physical crossing. An empty inner array is an
     * unconditional alternative. Arrays are preserved exactly, never unioned. */
    quest_gate_alternatives: unknown[][];
};
export type ValidatedProjection = { entries: ProjectionEntry[] };

function projectionTile(value: unknown, label: string): ProjectionTile {
    if (!isRecord(value) || Object.keys(value).sort().join(',') !== 'level,x,z') {
        throw new Error(`prereq_steps: ${label} must be exactly {x,z,level}`);
    }
    for (const axis of ['x', 'z', 'level'] as const) {
        if (!Number.isInteger(value[axis])) throw new Error(`prereq_steps: ${label}.${axis} is not an integer`);
    }
    const level = value.level as number;
    if (level < 0 || level > 3) throw new Error(`prereq_steps: ${label}.level ${level} is not a game plane`);
    return { x: value.x as number, z: value.z as number, level };
}

function projectionGate(value: unknown, label: string): unknown {
    if (!isRecord(value) || Object.keys(value).length !== 1) {
        throw new Error(`prereq_steps: ${label} is not a QuestGate (exactly one of Complete/Window)`);
    }
    if ('Complete' in value) {
        if (typeof value.Complete !== 'string' || value.Complete.length === 0) {
            throw new Error(`prereq_steps: ${label} Complete gate is malformed`);
        }
        return value;
    }
    if ('Window' in value) {
        const window = value.Window;
        if (!isRecord(window) || Object.keys(window).sort().join(',') !== 'quest,signal,values') {
            throw new Error(`prereq_steps: ${label} Window gate is malformed`);
        }
        if (typeof window.quest !== 'string' || window.quest.length === 0
            || typeof window.signal !== 'string' || window.signal.length === 0) {
            throw new Error(`prereq_steps: ${label} Window gate identity is malformed`);
        }
        if (!isRecord(window.values) || Object.keys(window.values).sort().join(',') !== 'max,min'
            || (window.values.min !== null && !Number.isInteger(window.values.min))
            || (window.values.max !== null && !Number.isInteger(window.values.max))) {
            throw new Error(`prereq_steps: ${label} Window gate values are malformed`);
        }
        return value;
    }
    throw new Error(`prereq_steps: ${label} is not a QuestGate (exactly one of Complete/Window)`);
}

function projectionGateAlternatives(value: unknown, label: string): unknown[][] {
    if (!Array.isArray(value)) throw new Error(`prereq_steps: ${label} quest_gate_alternatives must be a list of gate lists`);
    return value.map((alternative: unknown, alternativeIndex: number) => {
        const alternativeLabel = `${label} quest_gate_alternatives[${alternativeIndex}]`;
        if (!Array.isArray(alternative)) throw new Error(`prereq_steps: ${alternativeLabel} must be a list of QuestGate`);
        alternative.forEach((gate: unknown, gateIndex: number) => projectionGate(gate, `${alternativeLabel}[${gateIndex}]`));
        return alternative as unknown[];
    });
}


function projectionIdentityKey(crossing: ProjectionCrossing): string {
    return JSON.stringify([
        crossing.loc_id, crossing.option,
        crossing.at.x, crossing.at.z, crossing.at.level,
        crossing.takeoff.x, crossing.takeoff.z, crossing.takeoff.level,
        crossing.to.x, crossing.to.z, crossing.to.level,
        crossing.kind,
    ]);
}

function tilesEqual(left: ProjectionTile, right: ProjectionTile): boolean {
    return left.x === right.x && left.z === right.z && left.level === right.level;
}

/** Travel heading of the actual takeoff->to step; null unless it is one cardinal tile. */
function headingOf(takeoff: ProjectionTile, to: ProjectionTile): string | null {
    if (takeoff.level !== to.level) return null;
    if (to.x === takeoff.x && to.z === takeoff.z + 1) return 'north';
    if (to.x === takeoff.x && to.z === takeoff.z - 1) return 'south';
    if (to.x === takeoff.x + 1 && to.z === takeoff.z) return 'east';
    if (to.x === takeoff.x - 1 && to.z === takeoff.z) return 'west';
    return null;
}

function requireEnv(name: string, label: string): string {
    const value = process.env[name];
    if (!value) throw new Error(`set ${name} to ${label}`);
    return value;
}

/**
 * Load and validate the CLI shared crossing projection for 289. Reject
 * unsupported schema, duplicate crossings, invalid fields and any blocked or
 * inadmissible takeoff the generator would need as legal.
 */
export function loadPrereqCrossingProjection(
    revision: number,
    overrides?: { projectionPath?: string },
): ValidatedProjection | null {
    if (revision !== 289) return null;
    const projectionPath = overrides?.projectionPath ?? requireEnv('GAME_DATA_289_PREREQ_CROSSINGS', 'the CLI shared 289 crossing projection JSON');
    let parsed: unknown;
    try {
        parsed = JSON.parse(fs.readFileSync(projectionPath, 'utf8'));
    } catch (error) {
        throw new Error(`prereq_steps: cannot read GAME_DATA_289_PREREQ_CROSSINGS ${projectionPath}: ${error instanceof Error ? error.message : error}`);
    }
    if (!isRecord(parsed) || parsed.schema !== PREREQ_CROSSING_PROJECTION_SCHEMA
        || !Array.isArray(parsed.crossings) || Object.keys(parsed).sort().join(',') !== 'crossings,schema') {
        throw new Error('prereq_steps: projection must be exactly {schema:2,crossings}');
    }
    const entries: ProjectionEntry[] = [];
    const seen = new Set<string>();
    parsed.crossings.forEach((raw: unknown, index: number) => {
        const label = `projection crossings[${index}]`;
        if (!isRecord(raw) || Object.keys(raw).sort().join(',') !== 'admissible,crossing,quest_gate_alternatives,standable') {
            throw new Error(`prereq_steps: ${label} must be exactly {crossing,standable,admissible,quest_gate_alternatives}`);
        }
        if (typeof raw.standable !== 'boolean' || typeof raw.admissible !== 'boolean') {
            throw new Error(`prereq_steps: ${label} standable/admissible must be booleans`);
        }
        const quest_gate_alternatives = projectionGateAlternatives(raw.quest_gate_alternatives, label);
        const crossing = raw.crossing;
        if (!isRecord(crossing) || Object.keys(crossing).sort().join(',') !== 'at,kind,loc_id,option,takeoff,to') {
            throw new Error(`prereq_steps: ${label}.crossing must be exactly {loc_id,option,at,takeoff,to,kind}`);
        }
        if (!Number.isInteger(crossing.loc_id)) throw new Error(`prereq_steps: ${label}.crossing.loc_id is not an integer`);
        if (!Number.isInteger(crossing.option) || (crossing.option as number) < 0 || (crossing.option as number) > 255) {
            throw new Error(`prereq_steps: ${label}.crossing.option is not a representable u8 identity`);
        }
        const entryKind: unknown = crossing.kind;
        if (typeof entryKind !== 'string' || !KNOWN_CROSSING_KINDS.has(entryKind)) {
            throw new Error(`prereq_steps: ${label}.crossing.kind is not a lower-case CrossingKind`);
        }
        const entry: ProjectionEntry = {
            crossing: {
                loc_id: crossing.loc_id as number,
                option: crossing.option as number,
                at: projectionTile(crossing.at, `${label}.crossing.at`),
                takeoff: projectionTile(crossing.takeoff, `${label}.crossing.takeoff`),
                to: projectionTile(crossing.to, `${label}.crossing.to`),
                kind: entryKind,
            },
            standable: raw.standable,
            admissible: raw.admissible,
            quest_gate_alternatives,
        };
        const key = projectionIdentityKey(entry.crossing);
        if (seen.has(key)) throw new Error(`prereq_steps: duplicate crossing identity in the projection (${label})`);
        seen.add(key);
        entries.push(entry);
    });
    return { entries };
}

function toCrossingWire(template: CrossingTemplate, entry: ProjectionEntry): CrossingWire {
    return {
        crossing: {
            loc_id: entry.crossing.loc_id,
            option: entry.crossing.option,
            at: { ...entry.crossing.at },
            takeoff: { ...entry.crossing.takeoff },
            to: { ...entry.crossing.to },
            kind: entry.crossing.kind,
        },
        pass: {
            message: template.pass.message,
            landing: { ...entry.crossing.to },
            proves: { ...template.pass.proves },
            source: template.pass.source,
        },
        delay: template.delay.source === undefined
            ? { ticks: template.delay.ticks }
            : { ticks: template.delay.ticks, source: template.delay.source },
        source: template.source,
    };
}

/**
 * Join authored templates with the validated projection. Every generated
 * coordinate (at, to, takeoff, landing) is copied from the pack entry; the
 * template supplies only the loc/option/kind/heading selector and the
 * source-pinned pass/delay provenance. Refuses absent selectors, illegal
 * (blocked/inadmissible) takeoffs, and any legal entry for a catalog-covered
 * (loc, option, kind) identity that no template binds (completeness: A1 emits
 * every distinct legal takeoff).
 */
function buildPrereqFacts(templates: AuthoredPrereqTemplates, projection: ValidatedProjection): PrereqStepFacts {
    const legal = projection.entries.filter((entry) => entry.standable && entry.admissible);
    const coveredTriples = new Set(templates.rows.flatMap((row) =>
        row.crossings.map((template) => `${template.loc_id}|${template.option}|${template.kind}`)));
    const coveredLocs = new Set(templates.rows.flatMap((row) => row.crossings.map((template) => template.loc_id)));
    const tripleKey = (entry: ProjectionEntry) =>
        `${entry.crossing.loc_id}|${entry.crossing.option}|${entry.crossing.kind}`;
    const consumed = new Set<string>();
    const rows: PrereqRowWire[] = templates.rows.map((row) => {
        const crossings: CrossingWire[] = [];
        for (const template of row.crossings) {
            const matches = legal.filter((entry) =>
                entry.crossing.loc_id === template.loc_id
                && entry.crossing.option === template.option
                && entry.crossing.kind === template.kind
                && (template.heading === undefined || headingOf(entry.crossing.takeoff, entry.crossing.to) === template.heading));
            if (template.heading === undefined) {
                if (matches.length === 0) {
                    throw new Error(`prereq_steps: ${row.id} loc ${template.loc_id} has no legal pack crossing (absent/mismatched/illegal projection)`);
                }
                const takeoffs = new Set(matches.map((entry) =>
                    `${entry.crossing.takeoff.x},${entry.crossing.takeoff.z},${entry.crossing.takeoff.level}`));
                if (takeoffs.size !== matches.length) {
                    throw new Error(`prereq_steps: ${row.id} loc ${template.loc_id} shares a legal takeoff across pack crossings`);
                }
                matches.sort((left, right) => projectionIdentityKey(left.crossing) < projectionIdentityKey(right.crossing) ? -1 : 1);
                for (const entry of matches) {
                    crossings.push(toCrossingWire(template, entry));
                    consumed.add(projectionIdentityKey(entry.crossing));
                }
            } else {
                if (matches.length !== 1) {
                    throw new Error(`prereq_steps: ${row.id} loc ${template.loc_id} heading ${template.heading} matches ${matches.length} legal pack crossings (expected exactly one)`);
                }
                crossings.push(toCrossingWire(template, matches[0]));
                consumed.add(projectionIdentityKey(matches[0].crossing));
            }
        }
        const { crossings: _templateCrossings, ...rest } = row;
        void _templateCrossings;
        return { ...rest, crossings } as PrereqRowWire;
    });
    for (const entry of legal) {
        if (coveredTriples.has(tripleKey(entry)) && !consumed.has(projectionIdentityKey(entry.crossing))) {
            throw new Error(`prereq_steps: legal pack crossing ${projectionIdentityKey(entry.crossing)} for a catalog-covered identity is bound by no template`);
        }
    }
    // Wall-door start binding: headings of one loc must separate by actual
    // start relative to the anchor (one takeoff on the anchor, one across).
    // This checks the pack geometry instead of trusting hand-edited sides.
    for (const loc of coveredLocs) {
        const headed = legal.filter((entry) => entry.crossing.loc_id === loc && consumed.has(projectionIdentityKey(entry.crossing))
            && templates.rows.some((row) => row.crossings.some((template) =>
                template.loc_id === loc && template.heading !== undefined
                && headingOf(entry.crossing.takeoff, entry.crossing.to) === template.heading)));
        if (headed.length > 1) {
            const onAnchor = headed.filter((entry) => tilesEqual(entry.crossing.takeoff, entry.crossing.at));
            if (onAnchor.length === 0 || onAnchor.length === headed.length) {
                throw new Error(`prereq_steps: loc ${loc} headings do not separate by start relative to the anchor`);
            }
        }
    }
    return { rows, journals: templates.journals };
}

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

function citedText(content: string, source: string, onlyFiles?: ReadonlySet<string>) {
    return parseSource(source).filter(({ file }) => !onlyFiles || onlyFiles.has(file)).map(({ file, lines }) => {
        const all = fs.readFileSync(path.join(content, file), 'utf8').split(/\r?\n/);
        for (const line of lines) if (line > all.length) throw new Error(`prereq_steps: ${file}:${line} is outside the source file`);
        return lines.map((line) => all[line - 1]).join('\n');
    }).join('\n');
}

function isProductionScript(file: string) {
    return !file.split(path.sep).some((segment) => segment === '_test' || segment === 'debug');
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
    const constants = new Map<string, number>();
    for (const file of walkContentFiles(path.join(content, 'scripts'), '.constant').filter(isProductionScript)) {
        const relative = path.relative(content, file);
        for (const raw of fs.readFileSync(file, 'utf8').split(/\r?\n/)) {
            const match = /^\s*\^([A-Za-z0-9_]+)\s*=\s*(-?\d+)\s*(?:\/\/.*)?$/.exec(raw);
            if (!match) continue;
            const value = integer(match[2], `${relative}: ^${match[1]}`);
            const previous = constants.get(match[1]);
            if (previous !== undefined && previous !== value) {
                throw new Error(`prereq_steps: conflicting ^${match[1]} constants (${previous}, ${value})`);
            }
            constants.set(match[1], value);
        }
    }
    return constants;
}

function normalizeJournalText(text: string) {
    return text.replace(/@[A-Za-z0-9_]+@/g, '').replace(/\|/g, ' ').replace(/\s+/g, ' ').trim().toLowerCase();
}

function journalRuleRange(
    source: string,
    signal: string,
    constants: ReadonlyMap<string, number>,
    label: string,
    proves: RangeWire,
    needles: readonly string[],
    boundedProof: () => RangeWire | null,
) {
    const pattern = new RegExp(`%${escapeRegExp(signal)}\\s*(<=|>=|==|=|<|>)\\s*\\^([A-Za-z0-9_]+)\\b`, 'g');
    const comparisons = [...source.matchAll(pattern)].flatMap((match) => {
        const name = match[2];
        const value = constants.get(name);
        if (value === undefined || !new RegExp(`\\^${escapeRegExp(name)}\\s*=\\s*${value}(?!\\d)`).test(source)) return [];
        return [{ operator: match[1], name, value, index: match.index!, length: match[0].length }];
    });
    const candidates = new Map<string, RangeWire>();
    for (const comparison of comparisons) {
        let range: RangeWire | null = null;
        if (comparison.operator === '=' || comparison.operator === '==') {
            range = { min: comparison.value, max: comparison.value };
        } else if (comparison.operator === '<' && comparison.value > 0) {
            range = { min: 0, max: comparison.value - 1 };
        } else if (comparison.operator === '<=') {
            range = { min: 0, max: comparison.value };
        }
        if (range) candidates.set(`${range.min}:${range.max}`, range);
    }
    if ([...candidates.values()].some((range) => equalRangeWire(range, proves))) return proves;

    const returnsBeforeNeedle = (comparison: typeof comparisons[number]) => {
        const afterGuard = source.slice(comparison.index + comparison.length);
        const returnAt = afterGuard.indexOf('return;');
        if (returnAt < 0) return false;
        const afterReturn = normalizeJournalText(afterGuard.slice(returnAt + 'return;'.length));
        return needles.some((needle) => afterReturn.includes(needle));
    };
    const lowerCuts = comparisons.filter((comparison) =>
        comparison.operator === '<' && returnsBeforeNeedle(comparison)
        && comparisons.some((equal) => (equal.operator === '=' || equal.operator === '==')
            && equal.name === comparison.name && equal.value === comparison.value
            && returnsBeforeNeedle(equal)));
    const upperCuts = comparisons.filter((comparison) =>
        comparison.operator === '>' && returnsBeforeNeedle(comparison));
    for (const lower of lowerCuts) for (const upper of upperCuts) {
        const range = { min: lower.value + 1, max: upper.value };
        if (range.min <= range.max && equalRangeWire(range, proves)) return range;
    }
    const bounded = boundedProof();
    if (bounded && equalRangeWire(bounded, proves)) return bounded;
    throw new Error(`prereq_steps: ${label} has no source-pinned stage range [${proves.min},${proves.max}]`);
}

/** Verify every journal rule against its page text, stage guard and cited constant. */
export function assertPrereqJournalPins(
    content: string,
    journals: readonly PrereqJournalWire[],
    constants: ReadonlyMap<string, number> = scriptConstants(content),
) {
    let writerDomains: ProgressWriterDomains | undefined;
    for (const journal of journals) {
        if (journal.establishes.quest !== journal.quest) {
            throw new Error(`prereq_steps: ${journal.id} journal page and stage window quest disagree`);
        }
        if (journal.establishes.signal.length === 0) {
            throw new Error(`prereq_steps: ${journal.id} has an incomplete journal stage window`);
        }
        const pageFiles = new Set(parseSource(journal.source).map(({ file }) => file));
        if (!citedText(content, journal.source).includes(`[if_button,questlist:${journal.quest}]`)) {
            throw new Error(`prereq_steps: ${journal.id} source is not the declared journal page`);
        }
        for (const [index, rule] of journal.rules.entries()) {
            const label = `${journal.id} rule ${index}`;
            if (!parseSource(rule.source).some(({ file }) => pageFiles.has(file))) {
                throw new Error(`prereq_steps: ${label} source is not from its journal page`);
            }
            const source = citedText(content, rule.source);
            const pageText = normalizeJournalText(citedText(content, rule.source, pageFiles));
            for (const needle of rule.all) {
                if (needle !== normalizeJournalText(needle)) {
                    throw new Error(`prereq_steps: ${label} needle is not normalized lowercase text`);
                }
                if (!pageText.includes(needle)) {
                    throw new Error(`prereq_steps: ${label} needle is not pinned by ${rule.source}`);
                }
            }
            journalRuleRange(source, journal.establishes.signal, constants, label, rule.proves, rule.all, () => {
                const blocks = [...pageFiles].flatMap((file) =>
                    parseRs2Blocks(file, stripComments(fs.readFileSync(path.join(content, file), 'utf8'))))
                    .filter((block) => block.kind === 'if_button' && block.name === `questlist:${journal.quest}`);
                if (blocks.length !== 1 || !isProductionScript(blocks[0].span.file)) return null;
                const block = blocks[0];
                const covered = new Set(parseSource(rule.source)
                    .filter(({ file }) => file === block.span.file).flatMap(({ lines }) => lines));
                if (block.body.some((line) => stripComment(line.text).trim() && !covered.has(line.line))) return null;
                // The structural reader does not model escaped source quotes.
                if (block.body.some((line) => line.text.includes('\\'))) return null;
                let nodes;
                try {
                    nodes = parseBody(block.body);
                } catch {
                    return null;
                }
                const pinnedConstants = new Map<string, number>();
                for (const match of source.matchAll(/^\s*\^([A-Za-z0-9_]+)\s*=\s*(-?\d+)\s*(?:\/\/.*)?$/gm)) {
                    const value = Number(match[2]);
                    if (constants.get(match[1]) === value) pinnedConstants.set(match[1], value);
                }
                writerDomains ??= captureProgressWriterDomains(content);
                return boundedJournalRange(
                    nodes,
                    journal.establishes.signal,
                    pinnedConstants,
                    writerDomains(journal.establishes.signal),
                    covered,
                    rule.all,
                );
            });
        }
        for (const rule of journal.rules) {
            if (journal.after.filter((after) => equalRangeWire(after.proves, rule.proves)).length !== 1) {
                throw new Error(`prereq_steps: ${journal.id} needs exactly one after rule for each journal proof`);
            }
        }
        if (!journal.after.some((after) => equalRangeWire(after.after, journal.establishes.values)
            && journal.rules.some((rule) => equalRangeWire(rule.proves, after.proves)))) {
            throw new Error(`prereq_steps: ${journal.id} establishment window has no matching after rule`);
        }
    }
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
            // CrossingId.at stays the loc anchor: it must equal the cited map
            // tile. Direction distinguishes identities by the exact legal
            // takeoff, never by a second anchor.
            if (!crossing.crossing.takeoff) {
                throw new Error(`prereq_steps: ${row.id} crossing ${crossing.crossing.loc_id} has no pack-derived takeoff`);
            }
            if (!KNOWN_CROSSING_KINDS.has(crossing.crossing.kind)) {
                throw new Error(`prereq_steps: ${row.id} crossing ${crossing.crossing.loc_id} has an unknown kind`);
            }
            if (placements.length !== 1 || placements[0].x !== crossing.crossing.at.x
                || placements[0].z !== crossing.crossing.at.z || placements[0].plane !== crossing.crossing.at.level) {
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
                // Adjacent-door geometry runs from the legal takeoff to the
                // landing, not from the anchor: the reverse crossing lands on
                // the anchor itself, so anchor->to would measure zero.
                const distance = Math.abs(crossing.crossing.takeoff.x - crossing.crossing.to.x) + Math.abs(crossing.crossing.takeoff.z - crossing.crossing.to.z);
                if (distance !== 1 || crossing.crossing.takeoff.level !== crossing.crossing.to.level) {
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

export function extractPrereqStepsFacts(
    revision: number,
    overrides?: { projectionPath?: string },
): PrereqStepFacts {
    if (revision !== 289) return { rows: [], journals: [] };
    const templates = loadAuthoredTemplates();
    const projection = loadPrereqCrossingProjection(revision, overrides);
    if (!projection) throw new Error('prereq_steps: 289 requires GAME_DATA_289_PREREQ_CROSSINGS');
    return buildPrereqFacts(templates, projection);
}

export function assertPrereqStepFacts(
    content: string,
    facts: PrereqStepFacts,
    questRows?: readonly { id: string; colour_authoritative: boolean; transmission: string; stage_domain: RangeWire | null }[],
) {
    textPins(content, facts.rows);
    const constants = scriptConstants(content);
    assertPrereqJournalPins(content, facts.journals, constants);
    verifyPreconditions(content, facts.rows, constants);
    verifyTalkEffects(content, facts.rows, constants);
    verifyPlacements(content, facts.rows);
    assertPrereqPreservation(content, facts.rows, questRows, facts.journals);
    for (const row of facts.rows) {
        const seen = new Set<string>();
        for (const crossing of row.crossings) {
            const key = JSON.stringify([
                crossing.crossing.loc_id, crossing.crossing.option,
                crossing.crossing.at, crossing.crossing.takeoff, crossing.crossing.to,
                crossing.crossing.kind,
            ]);
            if (seen.has(key)) throw new Error(`prereq_steps: duplicate crossing identity in ${row.id}`);
            seen.add(key);
        }
        if (row.crossings.some((crossing) => crossing.delay.ticks < 0 || crossing.delay.ticks > 65535)) {
            throw new Error(`prereq_steps: invalid crossing delay in ${row.id}`);
        }
    }
}

