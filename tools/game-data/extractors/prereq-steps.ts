import fs from 'node:fs';
import path from 'node:path';

export const PREREQ_STEPS_SCHEMA = 3;
export type RangeWire = { min: number; max: number };
export type TileWire = { x: number; z: number; level: number };
export type CrossingWire = {
    crossing: { loc_id: number; option: number; at: TileWire; takeoff: TileWire; to: TileWire; kind: string };
    pass: { message: string; landing: TileWire; proves: RangeWire };
    delay: { ticks: number };
};
export type PrereqRowWire = {
    id: string;
    crossings: CrossingWire[];
    establishes: { quest: string; signal: string; values: RangeWire };
    after: { proves: RangeWire; after: RangeWire }[];
    probe: { refuse: { page: string; then: string | { precondition: unknown }; proves: RangeWire }[] };
    target: { npc: number; op: number; anchor: TileWire; leash: number };
    preconditions: unknown[];
    talk: {
        strict: boolean;
        prefer: string[];
        line_rules: { page: string; choice: string }[];
        branches: {
            opener: string;
            outcome: string;
            effect_page?: string;
            terminal?: string;
            proves: RangeWire;
            requires_at_opener?: { kind: string; count: number; otherwise: string };
            exit?: string;
        }[];
        repeat_after?: { reclaimed?: number };
    };
    memory: string;
    consent: string;
};
export type PrereqJournalRuleWire = { all: string[]; proves: RangeWire };
export type PrereqJournalWire = {
    id: string;
    quest: string;
    establishes: { quest: string; signal: string; values: RangeWire };
    after: { proves: RangeWire; after: RangeWire }[];
    rules: PrereqJournalRuleWire[];
};
export type PrereqStepFacts = { rows: PrereqRowWire[]; journals: PrereqJournalWire[] };

const AUTHORED_FILE = path.resolve(import.meta.dirname, '../prereq-steps.json');

/**
 * Authored crossing template. The template keeps the loc/option/kind selector
 * plus the authored pass/delay values, but must
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
    pass: { message: string; proves: RangeWire };
    delay: { ticks: number };
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
            if (keys !== 'delay,kind,loc_id,option,pass' && keys !== 'delay,heading,kind,loc_id,option,pass') {
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
                || !isRecord(template.pass.proves)
                || Object.keys(template.pass).sort().join(',') !== 'message,proves') {
                throw new Error(`prereq_steps: ${(row.id as string) ?? '?'} crossing template pass is malformed`);
            }
            if (!isRecord(template.delay) || !Number.isInteger(template.delay.ticks)
                || (template.delay.ticks as number) < 0 || (template.delay.ticks as number) > 65535
                || Object.keys(template.delay).join(',') !== 'ticks') {
                throw new Error(`prereq_steps: ${(row.id as string) ?? '?'} crossing template delay is malformed`);
            }
        }
    }
    for (const value of parsed.journals) {
        if (!isRecord(value) || Object.keys(value).sort().join(',') !== 'after,establishes,id,quest,rules'
            || typeof value.id !== 'string' || value.id.length === 0
            || typeof value.quest !== 'string' || value.quest.length === 0
            || !isRecord(value.establishes) || Object.keys(value.establishes).sort().join(',') !== 'quest,signal,values'
            || typeof value.establishes.quest !== 'string' || value.establishes.quest.length === 0
            || typeof value.establishes.signal !== 'string' || value.establishes.signal.length === 0
            || !isRangeWire(value.establishes.values)
            || !Array.isArray(value.after) || !Array.isArray(value.rules) || value.rules.length === 0) {
            throw new Error('prereq_steps: authored journal is malformed');
        }
        for (const rule of value.after) {
            if (!isRecord(rule) || Object.keys(rule).sort().join(',') !== 'after,proves'
                || !isRangeWire(rule.proves) || !isRangeWire(rule.after)) {
                throw new Error(`prereq_steps: ${value.id} journal after rule is malformed`);
            }
        }
        for (const rule of value.rules) {
            if (!isRecord(rule) || Object.keys(rule).sort().join(',') !== 'all,proves'
                || !Array.isArray(rule.all) || rule.all.length === 0
                || !rule.all.every((needle) => typeof needle === 'string' && needle.trim().length > 0)
                || !isRangeWire(rule.proves)) {
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
export const PREREQ_CROSSING_PROJECTION_SCHEMA = 3;
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
export type ValidatedProjection = {
    entries: ProjectionEntry[];
    identical_edge_groups: string[][];
};

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
        || !Array.isArray(parsed.crossings)
        || Object.keys(parsed).sort().join(',') !== 'crossings,identical_edge_groups,schema') {
        throw new Error('prereq_steps: projection must be exactly {schema:3,crossings,identical_edge_groups}');
    }
    if (!Array.isArray(parsed.identical_edge_groups) || parsed.identical_edge_groups.some((group: unknown) =>
        !Array.isArray(group) || group.length < 2
        || group.some((source: unknown) => typeof source !== 'string' || source.length === 0))) {
        throw new Error('prereq_steps: identical_edge_groups must contain source-row string groups');
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
    return { entries, identical_edge_groups: parsed.identical_edge_groups as string[][] };
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
        },
        delay: { ticks: template.delay.ticks },
    };
}

/**
 * Join authored templates with the validated projection. Every generated
 * coordinate (at, to, takeoff, landing) is copied from the pack entry; the
 * template supplies only the loc/option/kind/heading selector and the
 * authored pass/delay values. Refuses absent selectors, illegal
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

/** Shape-only checks on the generated facts; nothing here reads game content. */
export function assertPrereqStepFacts(facts: PrereqStepFacts) {
    for (const row of facts.rows) {
        const stage = row.establishes.values;
        if (stage.min !== stage.max
            || !row.talk.branches.some((branch) => branch.outcome === 'advice' || branch.outcome === 'start')) {
            throw new Error(`prereq_steps: ${row.id} has no exact authored stage-setting branch`);
        }
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
    for (const journal of facts.journals) {
        if (journal.establishes.quest !== journal.quest) {
            throw new Error(`prereq_steps: ${journal.id} journal page and stage window quest disagree`);
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

