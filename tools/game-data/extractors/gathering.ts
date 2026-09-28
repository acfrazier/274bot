// M-306 owns gathering extraction. The existing output is preserved until family cutover.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { integer, parseRows, parsePack, parseMapsquarePath, parseJm2LocPlacements, worldFromMapsquare, requireGatherText, placementMapInputs, PLACEMENT_MAPS_DIRECTORY, sha256 } from './common.ts';
export const gatherContentFiles = [
    'scripts/skill_mining/configs/mine.dbrow',
    'scripts/skill_mining/configs/rocks.loc',
    'scripts/skill_woodcutting/configs/trees.dbrow',
    'scripts/skill_woodcutting/configs/trees/achey.loc',
    'scripts/skill_woodcutting/configs/trees/burnt.loc',
    'scripts/skill_woodcutting/configs/trees/hollow.loc',
    'scripts/skill_woodcutting/configs/trees/magic.loc',
    'scripts/skill_woodcutting/configs/trees/maple.loc',
    'scripts/skill_woodcutting/configs/trees/normal.loc',
    'scripts/skill_woodcutting/configs/trees/oak.loc',
    'scripts/skill_woodcutting/configs/trees/willow.loc',
    'scripts/skill_woodcutting/configs/trees/yew.loc',
    'scripts/skill_fishing/configs/fishing.npc',
];
type GatherLocRef = { alias: string; id: number };
type GatherOutputRef = { alias: string; id: number };
type GatherConfigSection = { params: Record<string, string>; [key: string]: string | Record<string, string> };

const WOOD_PUBLICATION: Record<string, 'published' | 'conditional' | 'unpublished'> = {
    normal: 'published',
    oak: 'published',
    willow: 'published',
    maple: 'published',
    yew: 'published',
    magic: 'published',
    achey: 'conditional',
    hollow: 'conditional',
    jungle: 'unpublished',
    burnt: 'unpublished',
};
const REVISION_ABSENT_ON_274 = [
    { alias: 'dungeon_tree_closed', other_pin_id: 5083 },
    { alias: 'karam_dungeon_exit', other_pin_id: 5084 },
];

export function parseConfigSections(text: string) {
    const sections = new Map<string, GatherConfigSection>();
    let current: GatherConfigSection | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            current = { params: {} };
            sections.set(line.slice(1, -1), current);
            continue;
        }
        if (!current) continue;
        const eq = line.indexOf('=');
        if (eq <= 0) continue;
        const key = line.slice(0, eq);
        const value = line.slice(eq + 1);
        if (key === 'param') {
            const comma = value.indexOf(',');
            const paramKey = comma < 0 ? value : value.slice(0, comma);
            const paramValue = comma < 0 ? '' : value.slice(comma + 1);
            if (paramKey) current.params[paramKey] = paramValue;
            continue;
        }
        if (typeof current[key] !== 'string') current[key] = value;
    }
    return sections;
}
function joinPack(pack: Map<string, number>, alias: string, label: string, packName: string) {
    const id = pack.get(alias);
    if (id === undefined) throw new Error(`${label}: failed join, ${packName} lacks ${alias}`);
    return { alias, id };
}

function assertSelectedColumn(sections: Map<string, GatherConfigSection>, key: string, label: string) {
    if (![...sections.values()].some((section) => Object.prototype.hasOwnProperty.call(section.params, key))) {
        throw new Error(`${label}: missing selected column ${key}`);
    }
}

function extractLocResources(
    rows: { name: string; values: Record<string, string[][]> }[],
    locPack: Map<string, number>,
    objPack: Map<string, number>,
    sections: Map<string, GatherConfigSection>,
    spec: { aliasKey: string; outputKey: string; levelKey: string; transformKey: string; wood: boolean },
) {
    return rows.map((parsed) => {
        const aliases = (parsed.values[spec.aliasKey] ?? []).map((row) => row[0]).filter((alias): alias is string => Boolean(alias));
        if (aliases.length === 0) throw new Error(`${parsed.name}: no loc aliases`);
        const levelRaw = parsed.values[spec.levelKey]?.[0]?.[0];
        if (levelRaw === undefined) throw new Error(`${parsed.name}: missing ${spec.levelKey}`);
        const locIds = aliases.map((alias) => joinPack(locPack, alias, parsed.name, 'loc.pack'));
        const missingTransform: string[] = [];
        const empty = new Map<string, GatherLocRef>();
        for (const alias of aliases) {
            const section = sections.get(alias);
            const next = section?.params?.[spec.transformKey];
            if (!next) {
                missingTransform.push(alias);
                continue;
            }
            const joined = joinPack(locPack, next, `${parsed.name} transform ${alias}`, 'loc.pack');
            empty.set(joined.alias, joined);
        }
        const outputAlias = parsed.values[spec.outputKey]?.[0]?.[0];
        const output = outputAlias ? joinPack(objPack, outputAlias, `${parsed.name} output`, 'obj.pack') : null;
        const partialSides: string[] = [];
        if (missingTransform.length > 0) partialSides.push('transform');
        if (!output) partialSides.push('output');
        const resourceKey = spec.wood ? woodKey(parsed.name) : parsed.values.ore_name?.[0]?.[0];
        if (!resourceKey) throw new Error(`${parsed.name}: missing resource key`);
        const row: {
            table: string;
            resource_key: string;
            loc_ids: GatherLocRef[];
            empty_ids: GatherLocRef[];
            output: GatherOutputRef | null;
            level: number;
            qualification: 'complete' | 'partial';
            partial_sides: string[];
            missing_transform: string[];
            publication?: 'published' | 'conditional' | 'unpublished';
        } = {
            table: parsed.name,
            resource_key: resourceKey,
            loc_ids: locIds,
            empty_ids: [...empty.values()].sort((a, b) => a.id - b.id || a.alias.localeCompare(b.alias)),
            output,
            level: integer(levelRaw, parsed.name),
            qualification: partialSides.length === 0 ? 'complete' : 'partial',
            partial_sides: partialSides,
            missing_transform: missingTransform,
        };
        if (spec.wood) {
            const publication = WOOD_PUBLICATION[resourceKey];
            if (!publication) throw new Error(`${parsed.name}: unknown wood publication ${resourceKey}`);
            row.publication = publication;
        }
        return row;
    });
}

function woodKey(table: string) {
    const suffix = '_tree_table';
    if (!table.endsWith(suffix) || table.length === suffix.length) throw new Error(`${table}: expected wood table key`);
    return table.slice(0, -suffix.length);
}

export function extractGatherMethodsFacts(content: string, revision: number) {
    const mineText = requireGatherText(content, 'scripts/skill_mining/configs/mine.dbrow');
    const rockText = requireGatherText(content, 'scripts/skill_mining/configs/rocks.loc');
    const treeText = requireGatherText(content, 'scripts/skill_woodcutting/configs/trees.dbrow');
    const treeLocTexts = gatherContentFiles
        .filter((relative) => relative.startsWith('scripts/skill_woodcutting/configs/trees/') && relative.endsWith('.loc'))
        .map((relative) => requireGatherText(content, relative));
    const fishingText = requireGatherText(content, 'scripts/skill_fishing/configs/fishing.npc');
    const locPack = parsePack(requireGatherText(content, 'pack/loc.pack'));
    const objPack = parsePack(requireGatherText(content, 'pack/obj.pack'));
    if (locPack.size === 0) throw new Error('pack/loc.pack: required file missing ids');
    if (objPack.size === 0) throw new Error('pack/obj.pack: required file missing ids');
    const rockSections = parseConfigSections(rockText);
    const treeSections = new Map<string, GatherConfigSection>();
    for (const text of treeLocTexts) {
        for (const [alias, section] of parseConfigSections(text)) treeSections.set(alias, section);
    }
    assertSelectedColumn(rockSections, 'next_loc_stage_mining', 'scripts/skill_mining/configs/rocks.loc');
    assertSelectedColumn(treeSections, 'next_loc_stage', 'scripts/skill_woodcutting/configs/trees');
    const mining = extractLocResources(parseRows(mineText), locPack, objPack, rockSections, {
        aliasKey: 'rock',
        outputKey: 'rock_output',
        levelKey: 'rock_level',
        transformKey: 'next_loc_stage_mining',
        wood: false,
    });
    const woods = extractLocResources(parseRows(treeText), locPack, objPack, treeSections, {
        aliasKey: 'tree',
        outputKey: 'product',
        levelKey: 'levelrequired',
        transformKey: 'next_loc_stage',
        wood: true,
    });
    const fishingSeen = new Set<string>();
    const fishing: {
        category: string;
        primary_op: string;
        pair_op: string | null;
        level: null;
        output: null;
        qualification: 'partial';
        partial_sides: string[];
    }[] = [];
    for (const [name, section] of parseConfigSections(fishingText)) {
        const primary = typeof section.op1 === 'string' ? section.op1 : '';
        if (!primary) throw new Error(`${name}: missing primary op`);
        const category = typeof section.category === 'string' && section.category ? section.category : 'unknown';
        const pair = typeof section.op3 === 'string' ? section.op3 : null;
        const signature = `${category}\0${primary}\0${pair ?? ''}`;
        if (fishingSeen.has(signature)) continue;
        fishingSeen.add(signature);
        const partialSides = category === 'unknown' ? ['category', 'level', 'output'] : ['level', 'output'];
        fishing.push({
            category,
            primary_op: primary,
            pair_op: pair,
            level: null,
            output: null,
            qualification: 'partial',
            partial_sides: partialSides,
        });
    }
    if (mining.length === 0 || woods.length === 0 || fishing.length === 0) {
        throw new Error('gather_methods: no extracted rows');
    }
    const coverage: {
        class: string;
        table?: string;
        resource_key?: string;
        alias?: string;
        on_revision?: number;
        other_pin_id?: number;
        copied?: boolean;
        reason: string;
    }[] = [];
    for (const wood of woods) {
        if (wood.publication === 'conditional') {
            coverage.push({ class: 'conditional', table: wood.table, resource_key: wood.resource_key, reason: 'no supported consumer' });
        } else if (wood.publication === 'unpublished') {
            coverage.push({ class: 'unpublished', table: wood.table, resource_key: wood.resource_key, reason: 'unpublished wood' });
        }
    }
    if (revision === 274) {
        for (const absent of REVISION_ABSENT_ON_274) {
            if (locPack.has(absent.alias)) continue;
            coverage.push({
                class: 'revision-absent',
                alias: absent.alias,
                on_revision: 274,
                other_pin_id: absent.other_pin_id,
                copied: false,
                reason: '289-only loc, not copied onto 274',
            });
        }
    }
    return { woods, mining, fishing, coverage };
}

/** The `gather_methods.woods` row shape this family consumes. */
type GatherPlacementSource = { resource_key: string; publication?: 'published' | 'conditional' | 'unpublished'; loc_ids: { alias: string; id: number }[] };

/** One world LOC placement of a published resource loc id. No local coords, shape, or angle. */
export type GatherPlacement = { loc_id: number; x: number; z: number; plane: number };

/** A gather family with no selected published set. Unknown is not empty rows. */
export type GatherPlacementCoverage = { class: string; family: string; reason: string };

/** Published-resource world placements. `coverage` records what this family does not publish. */
export type GatherPlacementsFacts = { rows: GatherPlacement[]; coverage: GatherPlacementCoverage[] };

/** Per-wood placement count. A zero-hit loc_id variant is allowed; a zero-hit wood is not. */
export type GatherPlacementWood = { resource_key: string; loc_ids: number; placements: number };

/**
 * Provenance identity for this family: every scanned `maps/*.jm2` as one digest over
 * their per-file digests, `pack/loc.pack`, and the published loc-id set. Distinct from
 * `content_inputs` because the maps tree is a directory, not one tracked file per row.
 */
export type GatherPlacementInputs = {
    maps_directory: string;
    maps: { files: number; bytes: number; sha256: string };
    loc_pack: { path: string; bytes: number; sha256: string };
    published_loc_ids: { count: number; sha256: string };
};

export type GatherPlacementExtract = { facts: GatherPlacementsFacts; inputs: GatherPlacementInputs; woods: GatherPlacementWood[] };
/**
 * World LOC placements of the published woods' loc ids, plus the provenance identity
 * that invalidates them. LOC-only and fail-closed: a malformed row, a missing or
 * truncated `maps/`, a badly named map, a missing map, or a published wood with no
 * hits throws. Stored rows are resource loc_ids only, never empty/stump ids; the one
 * local coordinate pair is converted with `worldFromMapsquare`.
 */
export function extractGatherPlacementsFacts(content: string, woods: GatherPlacementSource[]): GatherPlacementExtract {
    const published = woods.filter((row) => row.publication === 'published');
    if (published.length === 0) throw new Error('gather_placements: no published woods');
    const locIds = new Set(published.flatMap((row) => row.loc_ids.map((loc) => loc.id)));
    const maps = placementMapInputs(content);
    const rows: GatherPlacement[] = [];
    const hits = new Map<number, number>();
    for (const input of maps) {
        const { mx, mz } = parseMapsquarePath(input.path);
        for (const placement of parseJm2LocPlacements(fs.readFileSync(path.join(content, input.path), 'utf8'), locIds)) {
            const world = worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane);
            rows.push({ loc_id: placement.loc_id, x: world.x, z: world.z, plane: world.plane });
            hits.set(placement.loc_id, (hits.get(placement.loc_id) ?? 0) + 1);
        }
    }
    rows.sort((a, b) => a.loc_id - b.loc_id || a.x - b.x || a.z - b.z || a.plane - b.plane);
    const perWood: GatherPlacementWood[] = [];
    for (const wood of published) {
        const placements = wood.loc_ids.reduce((sum, loc) => sum + (hits.get(loc.id) ?? 0), 0);
        if (placements === 0) throw new Error(`gather_placements: published wood ${wood.resource_key} has no LOC placements`);
        perWood.push({ resource_key: wood.resource_key, loc_ids: wood.loc_ids.length, placements });
    }
    const locPack = 'pack/loc.pack';
    const locPackFile = path.join(content, locPack);
    if (!fs.existsSync(locPackFile)) throw new Error(`${locPack}: required file missing`);
    return {
        facts: {
            rows,
            coverage: [{ class: 'unknown', family: 'mining', reason: 'no selected published-ore set' }],
        },
        inputs: {
            maps_directory: PLACEMENT_MAPS_DIRECTORY,
            maps: {
                files: maps.length,
                bytes: maps.reduce((sum, input) => sum + input.bytes, 0),
                sha256: crypto.createHash('sha256').update(maps.map((input) => `${input.sha256}  ${input.path}`).join('\n')).digest('hex'),
            },
            loc_pack: { path: locPack, ...sha256(locPackFile) },
            published_loc_ids: {
                count: locIds.size,
                sha256: crypto.createHash('sha256').update([...new Set(published.flatMap((row) => row.loc_ids.map((loc) => `${row.resource_key}\t${loc.id}`)))].sort().join('\n')).digest('hex'),
            },
        },
        woods: perWood,
    };
}
